use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{Api, api::ListParams, core::Selector};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{BytesBody, ExecIn, HttpForwarderService, retry_request};

/// The selector of the pods managed by the statefulset
fn pod_selector(sts: &StatefulSet) -> anyhow::Result<Selector> {
    let name = sts.metadata.name.as_deref().unwrap_or_default();
    let spec = sts
        .spec
        .as_ref()
        .ok_or(anyhow::anyhow!("statefulset {} does not have a spec", name))?;

    let selector = Selector::try_from(spec.selector.clone())
        .map_err(|e| anyhow::anyhow!("invalid selector of statefulset {}: {}", name, e))?;

    // an empty selector would match every pod in the namespace
    if selector.selects_all() {
        anyhow::bail!("statefulset {} has an empty selector", name);
    }

    Ok(selector)
}

/// List the vault pods of the statefulset
pub fn list_vault_pods(sts: &StatefulSet) -> anyhow::Result<ListParams> {
    Ok(ListParams::default().labels_from(&pod_selector(sts)?))
}

/// List the vault pods of the statefulset that are in the given state
pub fn list_vault_pods_in(
    sts: &StatefulSet,
    flavor: Flavor,
    exec_in: ExecIn,
) -> anyhow::Result<ListParams> {
    let mut selector = pod_selector(sts)?;
    selector.extend([exec_in.to_expression(&flavor.to_string())]);

    Ok(ListParams::default().labels_from(&selector))
}

/// Check if the vault pod is sealed based on its labels
/// Returns an error if the pod does not have the expected labels
pub fn is_sealed(pod: &Pod, flavor: &str) -> anyhow::Result<bool> {
    match pod.metadata.labels.as_ref() {
        None => Err(anyhow::anyhow!("pod does not have labels")),
        Some(labels) => match labels.get(&format!("{}-sealed", flavor)) {
            Some(x) if x.as_str() == "true" => Ok(true),
            Some(x) if x.as_str() == "false" => Ok(false),
            _ => Err(anyhow::anyhow!(
                "pod does not have a {}-sealed label",
                flavor
            )),
        },
    }
}

/// Check if the vault pod is active based on its labels
/// Returns an error if the pod does not have the expected labels
pub fn is_active(pod: &Pod, flavor: &str) -> anyhow::Result<bool> {
    match pod.metadata.labels.as_ref() {
        None => Err(anyhow::anyhow!("pod does not have labels")),
        Some(labels) => match labels.get(&format!("{}-active", flavor)) {
            Some(x) if x.as_str() == "true" => Ok(true),
            Some(x) if x.as_str() == "false" => Ok(false),
            _ => Err(anyhow::anyhow!(
                "pod does not have a {}-active label",
                flavor
            )),
        },
    }
}

/// Wrapper around the kube::Api type for the Vault pod
#[derive(Clone)]
pub struct PodApi {
    pub api: Api<Pod>,
    tls: bool,
    domain: String,
    pub flavor: Flavor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    OpenBao,
    Vault,
}

impl std::fmt::Display for Flavor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Flavor::OpenBao => write!(f, "openbao"),
            Flavor::Vault => write!(f, "vault"),
        }
    }
}

impl std::str::FromStr for Flavor {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openbao" => Ok(Flavor::OpenBao),
            "vault" => Ok(Flavor::Vault),
            _ => Err(anyhow::anyhow!("invalid flavor: {}", s)),
        }
    }
}

impl Flavor {
    pub fn container_name(&self) -> String {
        self.to_string()
    }
}

impl PodApi {
    pub fn new(api: Api<Pod>, tls: bool, domain: String, flavor: Flavor) -> Self {
        Self {
            api,
            tls,
            domain,
            flavor,
        }
    }
}

impl PodApi {
    /// Get a stream to a port on a pod
    /// The stream can be used to send HTTP requests
    pub async fn portforward(
        &self,
        pod: &str,
        port: u16,
    ) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + use<>> {
        let ports = [port];
        let mut pf = retry_request(&format!("forwarding port {} of pod {}", port, pod), || {
            self.api.portforward(pod, &ports)
        })
        .await?;
        pf.take_stream(port).ok_or(anyhow::anyhow!(
            "port {} is not available on pod {}",
            port,
            pod
        ))
    }

    pub async fn http(
        &self,
        pod: &str,
        port: u16,
    ) -> anyhow::Result<HttpForwarderService<BytesBody>> {
        let pf = self.portforward(pod, port).await?;

        if self.tls {
            return HttpForwarderService::https(&self.domain, pf).await;
        }

        HttpForwarderService::http(pf).await
    }
}

/// Wrapper around the kube::Api type for the Vault statefulset
pub struct StatefulSetApi {
    pub api: Api<StatefulSet>,
    pub flavor: Flavor,
}

impl StatefulSetApi {
    pub fn new(api: Api<StatefulSet>, flavor: Flavor) -> Self {
        Self { api, flavor }
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::apps::v1::StatefulSet;

    use crate::{ExecIn, Flavor, list_vault_pods, list_vault_pods_in};

    async fn statefulset() -> StatefulSet {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}.yaml",
            "apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets/vault-mgmt-e2e-2274"
        ))
        .await
        .unwrap();

        serde_saphyr::from_str(&file).unwrap()
    }

    #[tokio::test]
    async fn lists_pods_by_statefulset_selector() {
        let lp = list_vault_pods(&statefulset().await).unwrap();

        assert_eq!(
            lp.label_selector.unwrap(),
            "app.kubernetes.io/instance=vault-mgmt-e2e-2274,app.kubernetes.io/name=vault,component=server"
        );
    }

    #[tokio::test]
    async fn lists_pods_in_state_by_statefulset_selector() {
        let lp = list_vault_pods_in(&statefulset().await, Flavor::Vault, ExecIn::Standby).unwrap();

        assert_eq!(
            lp.label_selector.unwrap(),
            "app.kubernetes.io/instance=vault-mgmt-e2e-2274,app.kubernetes.io/name=vault,component=server,vault-active=false"
        );
    }

    #[tokio::test]
    async fn refuses_empty_selector() {
        let mut sts = statefulset().await;
        sts.spec.as_mut().unwrap().selector = Default::default();

        assert!(list_vault_pods(&sts).is_err());
        assert!(list_vault_pods_in(&sts, Flavor::Vault, ExecIn::Active).is_err());
    }
}
