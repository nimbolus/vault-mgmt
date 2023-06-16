use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{api::DeleteParams, runtime::wait::conditions::is_pod_running};
use secrecy::Secret;
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};
use tracing::*;

use crate::{
    ExecIn, StepDown, Unseal, VaultVersion, VAULT_PORT,
    {is_pod_ready, is_pod_standby, is_pod_unsealed}, {is_seal_status_initialized, GetSealStatus},
    {is_sealed, list_vault_pods, PodApi, StatefulSetApi, LABEL_KEY_VAULT_ACTIVE},
};

impl PodApi {
    /// Check if the vault pod has the specified version
    pub fn is_current(pod: &Pod, target: &VaultVersion) -> anyhow::Result<bool> {
        let pod_version = VaultVersion::try_from(pod)?;
        Ok(&pod_version == target)
    }

    /// Upgrade a vault pod
    ///
    ///  - a.1. if Pod version is outdated
    ///     - a.1.1. Delete pod
    ///     - a.1.2. Wait for pod to be deleted
    ///     - a.1.3. Wait for pod to be running
    ///  - a.2. if Pod version is current
    ///     - a.2.1. Pod is sealed
    ///         - a.2.1.1 Unseal pod
    ///     - a.2.2. Wait for pod to be unsealed
    ///     - a.2.3. Wait for pod to be ready
    pub async fn upgrade(
        &self,
        pod: Pod,
        target: &VaultVersion,
        token: Secret<String>,
        should_unseal: bool,
        force_upgrade: bool,
        keys: &[Secret<String>],
    ) -> anyhow::Result<()> {
        let name = pod
            .metadata
            .name
            .as_ref()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;

        // if Pod version is outdated (or upgrade is forced)
        if !Self::is_current(&pod, target)? || force_upgrade {
            // if Pod is active
            if pod
                .metadata
                .labels
                .ok_or(anyhow::anyhow!("pod does not have labels"))?
                .get(LABEL_KEY_VAULT_ACTIVE)
                .ok_or(anyhow::anyhow!(
                    "pod does not have an {} label",
                    LABEL_KEY_VAULT_ACTIVE
                ))?
                .as_str()
                == "true"
            {
                // Step down active pod
                self.http(name, VAULT_PORT).await?.step_down(token).await?;

                // Wait for other pod to take over
                kube::runtime::wait::await_condition(self.api.clone(), name, is_pod_standby())
                    .await?;
            }

            // Delete pod
            kube::runtime::wait::delete::delete_and_finalize(
                self.api.clone(),
                name,
                &DeleteParams::default(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("deleting pod {}: {}", name, e.to_string()))?;
        }

        // Wait for pod to be running
        kube::runtime::wait::await_condition(self.api.clone(), name, is_pod_running())
            .await
            .map_err(|e| {
                anyhow::anyhow!("waiting for pod {} to be running: {}", name, e.to_string())
            })?;

        let pod = self.api.get(name).await?;

        let mut pf = Retry::spawn(
            ExponentialBackoff::from_millis(50).map(jitter).take(5),
            || async move { self.http(name, VAULT_PORT).await },
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "attempting to forward http requests to {}: {}",
                name,
                e.to_string()
            )
        })?;

        pf.await_seal_status(is_seal_status_initialized())
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "waiting for pod to have required seal status {}: {}",
                    name,
                    e.to_string()
                )
            })?;

        if Self::is_current(&pod, target)? {
            // Pod is sealed
            if is_sealed(&pod)? {
                if should_unseal {
                    // Unseal pod
                    pf.unseal(keys).await.map_err(|e| {
                        anyhow::anyhow!("unsealing pod {}: {}", name, e.to_string())
                    })?;
                } else {
                    info!("pod {} is sealed, waiting for external unseal", name);
                }
            }
            // Wait for pod to be unsealed
            kube::runtime::wait::await_condition(self.api.clone(), name, is_pod_unsealed()).await?;
            // Wait for pod to be ready
            kube::runtime::wait::await_condition(self.api.clone(), name, is_pod_ready()).await?;
        }

        Ok(())
    }
}

impl StatefulSetApi {
    /// Upgrade a vault cluster
    ///
    /// - Verify that the statefulset is ready to be upgraded or in the process of being upgraded
    ///     - if statefulset is ready and all pods are ready, initialized and unsealed
    ///         - start upgrade process
    /// - Detect the target version from statefulset
    /// - Repeat for all standby pods
    ///     - Do a.1
    ///     - Do a.2
    /// - Upgrade active pods
    ///     - if Pod version is outdated
    ///         - Step down active pod
    ///         - Wait for other pod to take over
    ///     - Do a.1
    ///     - Do a.2
    ///
    /// - a.1. if Pod version is outdated
    ///     - a.1.1. Delete pod
    ///     - a.1.2. Wait for pod to be deleted
    ///     - a.1.3. Wait for pod to be running
    /// - a.2. if Pod version is current
    ///     - a.2.1. Pod is sealed
    ///         - a.2.1.1 Unseal pod
    ///     - a.2.2. Wait for pod to be unsealed
    ///     - a.2.3. Wait for pod to be ready
    pub async fn upgrade(
        &self,
        sts: StatefulSet,
        pods: &PodApi,
        token: Secret<String>,
        should_unseal: bool,
        force_upgrade: bool,
        keys: &[Secret<String>],
    ) -> anyhow::Result<()> {
        let target = VaultVersion::try_from(&sts)?;

        let standby = pods
            .api
            .list(&list_vault_pods().labels(&ExecIn::Standby.to_label_selector()))
            .await?;

        if standby.items.is_empty() {
            warn!("no standby pods found, skipping upgrade");
            return Ok(());
        }

        let active = pods
            .api
            .list(&list_vault_pods().labels(&ExecIn::Active.to_label_selector()))
            .await?;

        if active.items.is_empty() {
            warn!("no active pods found, skipping upgrade");
            return Ok(());
        }

        info!("upgrading standby pods");
        for pod in standby.iter() {
            pods.upgrade(
                pod.clone(),
                &target,
                token.clone(),
                should_unseal,
                force_upgrade,
                keys,
            )
            .await?;
        }

        info!("upgrading active pods");
        for pod in active.iter() {
            pods.upgrade(
                pod.clone(),
                &target,
                token.clone(),
                should_unseal,
                force_upgrade,
                keys,
            )
            .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use http::{Request, Response, StatusCode};
    use hyper::Body;
    use k8s_openapi::{api::core::v1::Pod, List};
    use kube::{Api, Client};
    use secrecy::Secret;
    use serde_yaml::Value;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use tower_test::mock::{self, Handle};

    use crate::{PodApi, VaultVersion};

    #[tokio::test]
    async fn is_current_returns_true_if_pod_version_is_current() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}{}.yaml",
            "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-", 0
        ))
        .await
        .unwrap();

        let pod: Pod = serde_yaml::from_str(&file).unwrap();

        let target = VaultVersion {
            version: "1.13.0".to_string(),
        };

        assert!(PodApi::is_current(&pod, &target).unwrap());
    }

    #[tokio::test]
    async fn is_current_returns_false_if_pod_version_is_outdated() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}{}.yaml",
            "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-", 0
        ))
        .await
        .unwrap();

        let pod: Pod = serde_yaml::from_str(&file).unwrap();

        let target = VaultVersion {
            version: "1.14.0".to_string(),
        };

        assert!(!PodApi::is_current(&pod, &target).unwrap());
    }

    #[tokio::test]
    async fn is_current_returns_false_if_pod_version_is_too_new() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}{}.yaml",
            "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-", 0
        ))
        .await
        .unwrap();

        let pod: Pod = serde_yaml::from_str(&file).unwrap();

        let target = VaultVersion {
            version: "1.0.0".to_string(),
        };

        assert!(!PodApi::is_current(&pod, &target).unwrap());
    }

    async fn mock_list_sealed(
        cancel: CancellationToken,
        handle: &mut Handle<Request<Body>, Response<Body>>,
    ) -> bool {
        let mut delete_called = false;
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
                        ("GET", "/api/v1/namespaces/vault-mgmt-e2e/pods", "&fieldSelector=metadata.name%3Dvault-mgmt-e2e-2274-1&resourceVersion=0", false) => {
                            let mut pod: Pod = serde_yaml::from_str(
                                &tokio::fs::read_to_string(format!(
                                    "tests/resources/installed/{}{}.yaml",
                                    "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-",
                                    1
                                ))
                                .await
                                .unwrap()
                            ).unwrap();

                            pod.metadata
                                .labels
                                .as_mut()
                                .unwrap()
                                .entry("vault-sealed".to_string())
                                .and_modify(|x| *x = "false".to_string());

                            let mut list = List::<Pod>::default();
                            list.items.push(pod);
                            list.metadata.resource_version = Some("0".to_string());
                            serde_json::to_string(&list).unwrap()
                        }
                        ("GET", "/api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-1", _, _) => {
                            let file =
                                tokio::fs::read_to_string(format!(
                                    "tests/resources/installed/{}.yaml",
                                    "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-1"
                                ))
                                .await
                                .unwrap();

                            serde_json::to_string(&serde_yaml::from_str::<Value>(&file).unwrap()).unwrap()
                        }
                        (method, _, _, _) => {
                            if method == "DELETE" {
                                delete_called = true;
                            }
                            send.send_response(Response::builder().status(StatusCode::NOT_FOUND).body(Body::from("404 not found")).unwrap());
                            continue;
                        },
                    };

                    send.send_response(Response::builder().body(Body::from(body)).unwrap());
                }
                _ = cancel.cancelled() => {
                    return delete_called;
                }
            }
        }
    }

    async fn setup() -> (Api<Pod>, JoinHandle<bool>, CancellationToken) {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();

        let cancel = CancellationToken::new();
        let cloned_token = cancel.clone();

        let spawned =
            tokio::spawn(async move { mock_list_sealed(cloned_token, &mut handle).await });

        let pods: Api<Pod> = Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));

        (pods, spawned, cancel)
    }

    #[tokio::test]
    async fn upgrade_does_not_delete_pod_if_current() {
        let target = VaultVersion {
            version: "1.13.0".to_string(),
        };

        let (api, service, cancel) = setup().await;

        let pods = PodApi::new(api, false, "vault-mgmt-e2e".to_string());

        let pod = pods.api.get("vault-mgmt-e2e-2274-1").await.unwrap();

        pods.upgrade(
            pod,
            &target,
            Secret::from_str("token").unwrap(),
            false,
            false,
            &[],
        )
        .await
        .unwrap_err();

        cancel.cancel();

        let delete_called = service.await.unwrap();

        assert!(!delete_called);
    }

    #[tokio::test]
    async fn upgrade_does_delete_pod_if_current_and_force_upgrade() {
        let target = VaultVersion {
            version: "1.13.0".to_string(),
        };

        let (api, service, cancel) = setup().await;

        let pods = PodApi::new(api, false, "vault-mgmt-e2e".to_string());

        let pod = pods.api.get("vault-mgmt-e2e-2274-1").await.unwrap();

        pods.upgrade(
            pod,
            &target,
            Secret::from_str("token").unwrap(),
            false,
            true,
            &[],
        )
        .await
        .unwrap_err();

        cancel.cancel();

        let delete_called = service.await.unwrap();

        assert!(delete_called);
    }
}
