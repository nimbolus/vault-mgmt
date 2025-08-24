use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{api::ListParams, Api};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{BytesBody, HttpForwarderService};

pub fn list_vault_pods(flavor: &str) -> ListParams {
    ListParams::default().labels(&format!("app.kubernetes.io/name={}", flavor))
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
                "pod does not have a {} label",
                &format!("{}-sealed", flavor)
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
                "pod does not have a {} label",
                &format!("{}-active", flavor)
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
    ) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
        let mut pf = self.api.portforward(pod, &[port]).await?;
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
