use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use secrecy::Secret;
use tracing::*;

use crate::{init_request, raft_join_request, BytesBody, HttpRequest, PodApi, VAULT_PORT};

#[derive(Debug, serde::Serialize)]
pub struct InitRequest {
    pub secret_shares: u8,
    pub secret_threshold: u8,
    pub stored_shares: u8,
    pub pgp_keys: serde_json::Value,
    pub recovery_shares: u8,
    pub recovery_threshold: u8,
    pub recovery_pgp_keys: serde_json::Value,
    pub root_token_pgp_key: String,
}

impl Default for InitRequest {
    fn default() -> Self {
        Self {
            secret_shares: 3,
            secret_threshold: 2,
            stored_shares: 0,
            pgp_keys: serde_json::Value::Null,
            recovery_shares: 0,
            recovery_threshold: 0,
            recovery_pgp_keys: serde_json::Value::Null,
            root_token_pgp_key: "".to_string(),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct InitResult {
    pub keys: Vec<Secret<String>>,
    pub keys_base64: Vec<Secret<String>>,
    pub root_token: Secret<String>,
}

/// Init a vault process
#[async_trait::async_trait]
pub trait Init {
    /// Init a vault process
    async fn init(&mut self, req: InitRequest) -> anyhow::Result<InitResult>;
}

#[async_trait::async_trait]
impl<T> Init for T
where
    T: HttpRequest<BytesBody> + Send + Sync + 'static,
{
    async fn init(&mut self, req: InitRequest) -> anyhow::Result<InitResult> {
        let body = serde_json::ser::to_string(&req)?;

        let http_req = init_request(Full::new(Bytes::from(body.to_string())).boxed())?;

        let (parts, body) = self.send_request(http_req).await?.into_parts();

        let body = String::from_utf8(body.to_vec())?;

        if parts.status != hyper::StatusCode::OK {
            return Err(anyhow::anyhow!("initializing: {}", body));
        }

        let response: InitResult = serde_json::from_str(&body)?;

        Ok(response)
    }
}

/// Join a vault process to a raft cluster
#[async_trait::async_trait]
pub trait RaftJoin {
    /// Join a vault process to a raft cluster
    async fn raft_join(&mut self, join_to: &str) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl<T> RaftJoin for T
where
    T: HttpRequest<BytesBody> + Send + Sync + 'static,
{
    async fn raft_join(&mut self, join_to: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "leader_api_addr": join_to,
        });

        let http_req = raft_join_request(Full::new(Bytes::from(body.to_string())).boxed())?;

        let (parts, body) = self.send_request(http_req).await?.into_parts();

        let body = String::from_utf8(body.to_vec())?;

        if parts.status != hyper::StatusCode::OK {
            return Err(anyhow::anyhow!("raft-joining: {}", body));
        }

        Ok(())
    }
}

#[tracing::instrument(skip_all)]
pub async fn init(domain: String, api: &Api<Pod>, pod_name: &str) -> anyhow::Result<InitResult> {
    let pod = api.get(pod_name).await?;

    info!("initializing: {}", pod_name);

    let pods = PodApi::new(api.clone(), true, domain);
    let mut pf = pods
        .http(
            pod.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("pod does not have a name"))?
                .as_str(),
            VAULT_PORT,
        )
        .await?;
    pf.ready().await?;

    let body = serde_json::json!({
        "secret_shares": 3,
        "secret_threshold": 2,
        "stored_shares": 0,
        "pgp_keys": serde_json::Value::Null,
        "recovery_shares": 0,
        "recovery_threshold": 0,
        "recovery_pgp_keys": serde_json::Value::Null,
        "root_token_pgp_key": "",
    });

    let http_req = Request::builder()
        .uri("/v1/sys/init")
        .header("Host", "127.0.0.1")
        .header("X-Vault-Request", "true")
        .method(hyper::Method::PUT)
        .body(Full::new(Bytes::from(body.to_string())).boxed())?;

    let (parts, body) = pf.send_request(http_req).await?.into_parts();

    let body = String::from_utf8(body.to_vec())?;

    if parts.status != hyper::StatusCode::OK {
        return Err(anyhow::anyhow!("{}", body));
    }

    let response: InitResult = serde_json::from_str(&body)?;

    Ok(response)
}

#[tracing::instrument(skip_all)]
pub async fn raft_join(
    domain: String,
    api: &Api<Pod>,
    pod_name: &str,
    join_to: &str,
) -> anyhow::Result<()> {
    let pod = api.get(pod_name).await?;

    info!(
        "raft joining: {} to {}",
        pod.metadata
            .name
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?,
        join_to,
    );

    let pods = PodApi::new(api.clone(), true, domain);
    let mut pf = pods
        .http(
            pod.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("pod does not have a name"))?
                .as_str(),
            VAULT_PORT,
        )
        .await?;
    pf.ready().await?;

    let body = serde_json::json!({
        "leader_api_addr": join_to,
    });

    let http_req = Request::builder()
        .uri("/v1/sys/storage/raft/join")
        .header("Host", "127.0.0.1")
        .header("X-Vault-Request", "true")
        .method(hyper::Method::POST)
        .body(Full::new(Bytes::from(body.to_string())).boxed())?;

    let (parts, body) = pf.send_request(http_req).await?.into_parts();

    let body = String::from_utf8(body.to_vec())?;

    if parts.status != hyper::StatusCode::OK {
        return Err(anyhow::anyhow!("{}", body));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use http::{Method, StatusCode};
    use wiremock::{
        matchers::{body_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::{
        HttpForwarderService, {Init, InitRequest, RaftJoin},
    };

    #[tokio::test]
    async fn init_calls_api() {
        let mock_server = MockServer::start().await;

        Mock::given(method(Method::PUT))
            .and(path("/v1/sys/init"))
            .and(header("X-Vault-Request", "true"))
            .respond_with(
                ResponseTemplate::new(StatusCode::OK).set_body_json(serde_json::json!({
                    "keys": vec!["abc"],
                    "keys_base64": vec!["YWJj"],
                    "root_token": "def",
                })),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let outcome = client.init(InitRequest::default()).await;

        assert!(outcome.is_ok());
    }

    #[tokio::test]
    async fn raft_join_calls_api() {
        let mock_server = MockServer::start().await;

        Mock::given(method(Method::POST))
            .and(path("/v1/sys/storage/raft/join"))
            .and(header("X-Vault-Request", "true"))
            .and(body_json(serde_json::json!({
                "leader_api_addr": "other-instance",
            })))
            .respond_with(ResponseTemplate::new(StatusCode::OK))
            .expect(1)
            .mount(&mock_server)
            .await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let outcome = client.raft_join("other-instance").await;

        assert!(outcome.is_ok());
    }
}
