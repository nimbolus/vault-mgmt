use hyper::Body;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use secrecy::{ExposeSecret, Secret};
use tokio::process::Command;

use crate::{
    list_vault_pods, ExecIn, {unseal_request, HttpRequest},
};

/// Get the unseal keys by running the specified command
#[tracing::instrument()]
pub async fn get_unseal_keys(key_cmd: String) -> anyhow::Result<Vec<Secret<String>>> {
    let output = Command::new("sh").arg("-c").arg(key_cmd).output().await?;

    let stdout = String::from_utf8(output.stdout)?;
    let keys = stdout
        .lines()
        .collect::<Vec<_>>()
        .iter()
        .map(|k| Secret::new(k.to_string()))
        .collect();

    Ok(keys)
}

/// List all pods that are sealed
pub async fn list_sealed_pods(api: &Api<Pod>) -> anyhow::Result<Vec<Pod>> {
    let pods = api
        .list(&list_vault_pods().labels(&ExecIn::Sealed.to_label_selector()))
        .await?;

    Ok(pods.items)
}

/// Unseal a vault process using the provided keys
#[async_trait::async_trait]
pub trait Unseal {
    /// Unseal a vault process using the provided keys
    async fn unseal(&mut self, keys: &[Secret<String>]) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl<T> Unseal for T
where
    T: HttpRequest + Send + Sync + 'static,
{
    async fn unseal(&mut self, keys: &[Secret<String>]) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Err(anyhow::anyhow!("no keys provided"));
        }

        for key in keys {
            self.ready().await?;

            let body = serde_json::json!({
                "key": key.expose_secret(),
                "reset": false,
                "migrate": false,
            });

            let http_req = unseal_request(Body::from(body.to_string()))?;

            let (parts, body) = self.send_request(http_req).await?.into_parts();

            let body = hyper::body::to_bytes(body).await?;
            let body = String::from_utf8(body.to_vec())?;

            if !(parts.status.is_success() || parts.status.is_redirection()) {
                return Err(anyhow::anyhow!("unsealing: {}", body));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use http::{Method, Request, Response, StatusCode};
    use hyper::Body;
    use k8s_openapi::{api::core::v1::Pod, List};
    use kube::{Api, Client};
    use secrecy::Secret;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use tower_test::mock::{self, Handle};
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::{list_sealed_pods, HttpForwarderService, Unseal};

    async fn mock_list_sealed(
        cancel: CancellationToken,
        handle: &mut Handle<Request<Body>, Response<Body>>,
    ) {
        loop {
            tokio::select! {
                request = handle.next_request() => {
                    let (request, send) = request.expect("Service not called");

                    let method = request.method().to_string();
                    let uri = request.uri().path().to_string();
                    let query = request.uri().query().unwrap_or_default().to_string();

                    let watch = query.contains("watch=true");

                    println!("{} {} {} ", method, uri, query);

                    let body = match (method.as_str(), uri.as_str(), query.as_str(), watch) {
                        ("GET", "/api/v1/namespaces/vault-mgmt-e2e/pods", "&labelSelector=vault-sealed%3Dtrue", false) => {
                            let mut list = List::<Pod>::default();

                            for id in 0..=2 {
                                let file = tokio::fs::read_to_string(format!(
                                    "tests/resources/installed/{}{}.yaml",
                                    "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-",
                                    id
                                ))
                                .await
                                .unwrap();

                                let pod: Pod = serde_yaml::from_str(&file).unwrap();
                                list.items.push(pod);
                            }

                            list.metadata.resource_version = Some(format!("{}", 1));

                            serde_json::to_string(&list).unwrap()
                        }
                        _ => panic!("Unexpected API request {:?} {:?} {:?}", method, uri, query),
                    };

                    send.send_response(Response::builder().body(Body::from(body)).unwrap());
                }
                _ = cancel.cancelled() => {
                    return;
                }
            }
        }
    }

    async fn setup() -> (Api<Pod>, JoinHandle<()>, CancellationToken) {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();

        let cancel = CancellationToken::new();
        let cloned_token = cancel.clone();

        let spawned = tokio::spawn(async move {
            mock_list_sealed(cloned_token, &mut handle).await;
        });

        let pods: Api<Pod> = Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));

        (pods, spawned, cancel)
    }

    #[tokio::test]
    async fn get_sealed_pods_returns_sealed_pods() {
        let (api, service, cancel) = setup().await;

        let pods = list_sealed_pods(&api).await.unwrap();

        assert_eq!(pods.len(), 3);

        cancel.cancel();

        service.await.unwrap();
    }

    #[tokio::test]
    async fn unseal_returns_err_without_keys() {
        let mock_server = MockServer::start().await;
        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let outcome = client.unseal(&vec![]).await;

        assert!(outcome.is_err());
    }

    struct UnsealBodyMatcher(String);

    impl wiremock::Match for UnsealBodyMatcher {
        fn matches(&self, request: &wiremock::Request) -> bool {
            let result: Result<serde_json::Value, _> = serde_json::from_slice(&request.body);
            if let Ok(body) = result {
                body.get("key").is_some()
                    && body.get("key").unwrap() == &self.0
                    && body.get("reset").is_some()
                    && body.get("migrate").is_some()
            } else {
                false
            }
        }
    }

    #[tokio::test]
    async fn unseal_calls_api() {
        let mock_server = MockServer::start().await;

        for key in ["abc".to_string(), "def".to_string(), "ghi".to_string()] {
            Mock::given(method(Method::PUT))
                .and(path("/v1/sys/unseal"))
                .and(header("X-Vault-Request", "true"))
                .and(UnsealBodyMatcher(key))
                .respond_with(ResponseTemplate::new(StatusCode::OK))
                .expect(1)
                .mount(&mock_server)
                .await;
        }

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let outcome = client
            .unseal(&vec![
                Secret::from_str("abc").unwrap(),
                Secret::from_str("def").unwrap(),
                Secret::from_str("ghi").unwrap(),
            ])
            .await;

        assert!(outcome.is_ok());
    }
}
