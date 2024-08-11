use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{api::ListParams, Api};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{BytesBody, HttpForwarderService};

pub const LABEL_KEY_VAULT_ACTIVE: &str = "vault-active";
pub const LABEL_KEY_VAULT_SEALED: &str = "vault-sealed";

pub fn list_vault_pods() -> ListParams {
    ListParams::default().labels("app.kubernetes.io/name=vault")
}

/// Check if the vault pod is sealed based on its labels
/// Returns an error if the pod does not have the expected labels
pub fn is_sealed(pod: &Pod) -> anyhow::Result<bool> {
    match pod.metadata.labels.as_ref() {
        None => Err(anyhow::anyhow!("pod does not have labels")),
        Some(labels) => match labels.get(LABEL_KEY_VAULT_SEALED) {
            Some(x) if x.as_str() == "true" => Ok(true),
            Some(x) if x.as_str() == "false" => Ok(false),
            _ => Err(anyhow::anyhow!(
                "pod does not have a {} label",
                LABEL_KEY_VAULT_SEALED
            )),
        },
    }
}

/// Check if the vault pod is active based on its labels
/// Returns an error if the pod does not have the expected labels
pub fn is_active(pod: &Pod) -> anyhow::Result<bool> {
    match pod.metadata.labels.as_ref() {
        None => Err(anyhow::anyhow!("pod does not have labels")),
        Some(labels) => match labels.get(LABEL_KEY_VAULT_ACTIVE) {
            Some(x) if x.as_str() == "true" => Ok(true),
            Some(x) if x.as_str() == "false" => Ok(false),
            _ => Err(anyhow::anyhow!(
                "pod does not have a {} label",
                LABEL_KEY_VAULT_ACTIVE
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
}

impl PodApi {
    pub fn new(api: Api<Pod>, tls: bool, domain: String) -> Self {
        Self { api, tls, domain }
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
}

impl From<Api<StatefulSet>> for StatefulSetApi {
    fn from(api: Api<StatefulSet>) -> Self {
        Self { api }
    }
}
