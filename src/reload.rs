use std::collections::HashMap;

use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::Api;
use tracing::*;

use crate::{ExecIn, Flavor, exec_pod, list_vault_pods, list_vault_pods_in, retry_request};

/// Shell script that sends SIGHUP to the server process and prints its pid.
///
/// The server is usually not pid 1: the helm chart starts it through `sh` and
/// `dumb-init`, and `sh` does not forward the signal. So the process is looked
/// up by its command line, `<binary> server ...`.
fn reload_script(binary: &str) -> String {
    format!(
        r#"for p in /proc/[0-9]*; do
  set -f
  set -- $(tr '\000' ' ' < "$p/cmdline" 2>/dev/null)
  if [ "${{1##*/}}" = "{binary}" ] && [ "$2" = "server" ]; then
    kill -HUP "${{p#/proc/}}" && echo "${{p#/proc/}}"
  fi
done"#
    )
}

/// Reload the configuration of the vault pods by sending SIGHUP to the server
/// process. This makes vault pick up renewed TLS certificates without a restart.
/// Only pods in the given state are reloaded, all pods if it is `None`.
/// Fails if no pod matches, unless `ignore_not_found` is set.
pub async fn reload(
    api: &Api<Pod>,
    sts: &StatefulSet,
    flavor: Flavor,
    reload_in: Option<ExecIn>,
    ignore_not_found: bool,
) -> anyhow::Result<()> {
    let lp = match reload_in {
        Some(reload_in) => list_vault_pods_in(sts, flavor, reload_in)?,
        None => list_vault_pods(sts)?,
    };
    let pods = retry_request("listing vault pods", || api.list(&lp)).await?;

    if pods.items.is_empty() {
        let msg = match reload_in {
            Some(reload_in) => format!("no {} vault pods found", reload_in),
            None => "no vault pods found".to_string(),
        };
        if ignore_not_found {
            info!("{}, nothing to reload", msg);
            return Ok(());
        }
        anyhow::bail!(msg);
    }

    for pod in pods.iter() {
        let name = pod
            .metadata
            .name
            .as_ref()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;

        let (stdout, stderr) = exec_pod(
            api,
            pod,
            reload_script(flavor.binary_name()),
            HashMap::new(),
        )
        .await?;

        let pids = stdout.split_whitespace().collect::<Vec<_>>();
        if pids.is_empty() {
            anyhow::bail!(
                "no {} server process reloaded in pod {}: {}",
                flavor.binary_name(),
                name,
                stderr.trim()
            );
        }

        info!(
            "reloaded {} (pid {}) in pod {}",
            flavor,
            pids.join(", "),
            name
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::reload_script;

    #[test]
    fn reload_script_is_valid_shell() {
        let status = Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(reload_script("vault"))
            .status()
            .unwrap();

        assert!(status.success());
    }

    #[test]
    fn reload_script_matches_server_binary() {
        let script = reload_script("bao");

        assert!(script.contains(r#"[ "${1##*/}" = "bao" ] && [ "$2" = "server" ]"#));
    }
}
