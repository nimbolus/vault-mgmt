use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{api::DeleteParams, runtime::wait::conditions::is_pod_running};
use secrecy::Secret;
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
        keys: &[Secret<String>],
    ) -> anyhow::Result<()> {
        let name = pod
            .metadata
            .name
            .as_ref()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;

        // if Pod version is outdated
        if !Self::is_current(&pod, target)? {
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
            .await?;
        }

        // Wait for pod to be running
        kube::runtime::wait::await_condition(self.api.clone(), name, is_pod_running()).await?;

        let pod = self.api.get(name).await?;
        let mut pf = self.http(name, VAULT_PORT).await?;

        pf.await_seal_status(is_seal_status_initialized()).await?;

        if Self::is_current(&pod, target)? {
            // Pod is sealed
            if is_sealed(&pod)? {
                // Unseal pod

                pf.unseal(keys).await?;
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
            pods.upgrade(pod.clone(), &target, token.clone(), keys)
                .await?;
        }

        info!("upgrading active pods");
        for pod in active.iter() {
            pods.upgrade(pod.clone(), &target, token.clone(), keys)
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::Pod;

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
}
