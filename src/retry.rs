use std::{fmt::Debug, future::Future, time::Duration};

use kube::{
    Api, Resource,
    runtime::{
        wait::{self, Condition, await_condition},
        watcher,
    },
};
use serde::de::DeserializeOwned;
use tokio::time::{Instant, sleep, timeout};
use tokio_retry::strategy::{ExponentialBackoff, jitter};
use tracing::*;

/// How long a single request or watch attempt may take before it is abandoned.
/// Connections to the Kubernetes API can die without being reset, which would
/// otherwise block forever as kube does not set a read timeout.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait for a condition in total before giving up.
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(600);

/// How long to pause between two polls of the Vault API.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How often a request is attempted before giving up. Resets come in bursts,
/// 8 attempts wait up to 16s in total.
const MAX_ATTEMPTS: usize = 8;

/// Delays between attempts: 200ms, 400ms, 800ms, ... capped at 5s, with jitter
fn backoff() -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(2)
        .factor(100)
        .max_delay(Duration::from_secs(5))
        .map(jitter)
}

/// Whether a request that failed with `err` may succeed when tried again.
/// Connection errors and overloaded or unavailable API servers are transient,
/// errors like missing permissions or objects are not.
pub fn is_transient(err: &kube::Error) -> bool {
    match err {
        kube::Error::HyperError(_) | kube::Error::Service(_) | kube::Error::ReadEvents(_) => true,
        kube::Error::Api(status) => is_transient_status(status.code),
        _ => false,
    }
}

fn is_transient_status(code: u16) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504)
}

fn is_transient_watch(err: &wait::Error) -> bool {
    let wait::Error::ProbeFailed(err) = err;
    match err {
        watcher::Error::InitialListFailed(e)
        | watcher::Error::WatchStartFailed(e)
        | watcher::Error::WatchFailed(e) => is_transient(e),
        // 410 Gone means the watch has to start over with a fresh list
        watcher::Error::WatchError(status) => {
            status.code == 410 || is_transient_status(status.code)
        }
        watcher::Error::NoResourceVersion => false,
    }
}

/// Run a request against the Kubernetes API, retrying transient failures
/// and attempts that do not complete within [`ATTEMPT_TIMEOUT`].
///
/// The request may be sent more than once, so it must be safe to repeat.
pub async fn retry_request<T, F, Fut>(what: &str, mut request: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, kube::Error>>,
{
    let mut delays = backoff().take(MAX_ATTEMPTS - 1);

    loop {
        let err = match timeout(ATTEMPT_TIMEOUT, request()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(e)) if !is_transient(&e) => return Err(e.into()),
            Ok(Err(e)) => anyhow::Error::from(e),
            Err(_) => anyhow::anyhow!("no response within {:?}", ATTEMPT_TIMEOUT),
        };

        match delays.next() {
            Some(delay) => {
                warn!("{} failed, retrying in {:?}: {}", what, delay, err);
                sleep(delay).await;
            }
            None => {
                return Err(err.context(format!("{} failed after {} attempts", what, MAX_ATTEMPTS)));
            }
        }
    }
}

/// Wait until the object `name` fulfills `cond`, like [`await_condition`], but
/// survive connection failures.
///
/// Each attempt watches for at most [`ATTEMPT_TIMEOUT`]. When it fails with a
/// transient error or runs out of time, a new attempt starts with a fresh list,
/// so changes missed on a dead connection are picked up. Gives up after
/// [`WAIT_TIMEOUT`].
pub async fn wait_for<K>(
    api: &Api<K>,
    name: &str,
    cond: impl Condition<K>,
) -> anyhow::Result<Option<K>>
where
    K: Clone + Debug + Send + DeserializeOwned + Resource + 'static,
{
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut delays = backoff();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("timed out after {:?} waiting for {}", WAIT_TIMEOUT, name);
        }

        let attempt = await_condition(api.clone(), name, |obj: Option<&K>| {
            cond.matches_object(obj)
        });

        match timeout(ATTEMPT_TIMEOUT.min(remaining), attempt).await {
            Ok(Ok(obj)) => return Ok(obj),
            Ok(Err(e)) if !is_transient_watch(&e) => {
                return Err(anyhow::Error::from(e).context(format!("waiting for {}", name)));
            }
            Ok(Err(e)) => {
                // the iterator never ends, max_delay keeps it going at 5s
                let delay = delays.next().unwrap_or(Duration::from_secs(5));
                warn!(
                    "waiting for {} failed, retrying in {:?}: {}",
                    name, delay, e
                );
                sleep(delay.min(deadline.saturating_duration_since(Instant::now()))).await;
            }
            // condition not met yet or the connection died, start over
            Err(_) => debug!("still waiting for {}, restarting watch", name),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use kube::core::Status;

    use super::*;

    fn api_error(code: u16) -> kube::Error {
        kube::Error::Api(Box::new(Status::failure("error", "Error").with_code(code)))
    }

    fn connection_reset() -> kube::Error {
        kube::Error::ReadEvents(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
    }

    #[test]
    fn classifies_errors() {
        assert!(is_transient(&connection_reset()));
        assert!(is_transient(&api_error(503)));
        assert!(is_transient(&api_error(429)));
        assert!(!is_transient(&api_error(403)));
        assert!(!is_transient(&api_error(404)));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_transient_errors() {
        let attempts = AtomicUsize::new(0);

        let result = retry_request("test", || async {
            match attempts.fetch_add(1, Ordering::SeqCst) {
                0 | 1 => Err(connection_reset()),
                _ => Ok("done"),
            }
        })
        .await;

        assert_eq!(result.unwrap(), "done");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn fails_immediately_on_permanent_errors() {
        let attempts = AtomicUsize::new(0);

        let result: anyhow::Result<()> = retry_request("test", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(api_error(403))
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_max_attempts() {
        let attempts = AtomicUsize::new(0);

        let result: anyhow::Result<()> = retry_request("test", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(connection_reset())
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_attempts_without_response() {
        let attempts = AtomicUsize::new(0);

        let result = retry_request("test", || async {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                // a dead connection that never answers
                std::future::pending::<()>().await;
            }
            Ok("done")
        })
        .await;

        assert_eq!(result.unwrap(), "done");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}
