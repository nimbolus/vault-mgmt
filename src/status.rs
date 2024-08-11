use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use kube::runtime::wait::Condition;
use secrecy::Secret;

use crate::{raft_configuration_request, seal_status_request, BytesBody, HttpRequest};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PodSealStatus {
    #[serde(rename = "type")]
    pub type_: String,
    pub initialized: bool,
    pub sealed: bool,
    pub t: u8,
    pub n: u8,
    pub progress: u8,
    pub nonce: String,
    pub version: String,
    pub build_date: String,
    pub migration: bool,
    pub recovery_seal: bool,
    pub storage_type: String,
    pub ha_enabled: Option<bool>,
    pub cluster_name: Option<String>,
    pub cluster_id: Option<String>,
    pub active_time: Option<String>,
    pub leader_address: Option<String>,
    pub leader_cluster_address: Option<String>,
    pub raft_committed_index: Option<u64>,
    pub raft_applied_index: Option<u64>,
}

/// Get vault pod's seal status
#[async_trait::async_trait]
pub trait GetSealStatus {
    /// Get vault pod's seal status
    async fn seal_status(&mut self) -> anyhow::Result<PodSealStatus>;

    /// Wait for vault pod's seal status to match the provided condition
    async fn await_seal_status(
        &mut self,
        cond: impl Condition<PodSealStatus> + Send,
    ) -> Result<Option<PodSealStatus>, anyhow::Error>;
}

#[async_trait::async_trait]
impl<T> GetSealStatus for T
where
    T: HttpRequest<BytesBody> + Send + Sync + 'static,
{
    async fn seal_status(&mut self) -> anyhow::Result<PodSealStatus> {
        let http_req = seal_status_request(Empty::<Bytes>::new().boxed())?;

        let (parts, body) = self.send_request(http_req).await?.into_parts();

        let body = String::from_utf8(body.to_vec())?;

        if parts.status != hyper::StatusCode::OK {
            return Err(anyhow::anyhow!("getting seal status: {}", body));
        }

        Ok(serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("{}: {}", e, body))?)
    }

    async fn await_seal_status(
        &mut self,
        cond: impl Condition<PodSealStatus> + Send,
    ) -> Result<Option<PodSealStatus>, anyhow::Error> {
        loop {
            let status = self.seal_status().await?;
            if cond.matches_object(Some(&status)) {
                return Ok(Some(status));
            }
        }
    }
}

#[must_use]
pub fn is_seal_status_initialized() -> impl Condition<PodSealStatus> {
    |obj: Option<&PodSealStatus>| {
        if let Some(status) = obj {
            return status.initialized;
        }
        false
    }
}

#[must_use]
pub fn is_seal_status_sealed() -> impl Condition<PodSealStatus> {
    |obj: Option<&PodSealStatus>| {
        if let Some(status) = obj {
            return status.sealed;
        }
        false
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaftConfiguration {
    pub request_id: String,
    pub lease_id: String,
    pub renewable: bool,
    pub lease_duration: u64,
    pub data: RaftConfigurationData,
    pub wrap_info: Option<serde_json::Value>,
    pub warnings: Option<serde_json::Value>,
    pub auth: Option<serde_json::Value>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaftConfigurationData {
    pub config: RaftConfigurationDataConfig,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaftConfigurationDataConfig {
    pub servers: Vec<RaftConfigurationServer>,
    pub index: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaftConfigurationServer {
    pub node_id: String,
    pub address: String,
    pub leader: bool,
    pub protocol_version: String,
    pub voter: bool,
}

/// Get vault pod's raft configuration
#[async_trait::async_trait]
pub trait GetRaftConfiguration {
    /// Get vault pod's raft configuration
    async fn raft_configuration(
        &mut self,
        token: Secret<String>,
    ) -> anyhow::Result<RaftConfiguration>;

    /// Wait for vault pod's raft configuration to match the provided condition
    async fn await_raft_configuration(
        &mut self,
        token: Secret<String>,
        cond: impl Condition<RaftConfiguration> + Send,
    ) -> Result<Option<RaftConfiguration>, anyhow::Error>;
}

#[async_trait::async_trait]
impl<T> GetRaftConfiguration for T
where
    T: HttpRequest<BytesBody> + Send + Sync + 'static,
{
    async fn raft_configuration(
        &mut self,
        token: Secret<String>,
    ) -> anyhow::Result<RaftConfiguration> {
        let http_req = raft_configuration_request(token, Empty::<Bytes>::new().boxed())?;

        let (parts, body) = self.send_request(http_req).await?.into_parts();

        let body = String::from_utf8(body.to_vec())?;

        if parts.status != hyper::StatusCode::OK {
            return Err(anyhow::anyhow!("getting raft configuration: {}", body));
        }

        Ok(serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("{}: {}", e, body))?)
    }

    async fn await_raft_configuration(
        &mut self,
        token: Secret<String>,
        cond: impl Condition<RaftConfiguration> + Send,
    ) -> Result<Option<RaftConfiguration>, anyhow::Error> {
        loop {
            let config = self.raft_configuration(token.clone()).await?;
            if cond.matches_object(Some(&config)) {
                return Ok(Some(config));
            }
        }
    }
}

#[must_use]
pub fn raft_configuration_any_leader() -> impl Condition<RaftConfiguration> {
    |obj: Option<&RaftConfiguration>| {
        if let Some(config) = obj {
            return config.data.config.servers.iter().any(|s| s.leader);
        }
        false
    }
}

#[must_use]
pub fn raft_configuration_all_voters() -> impl Condition<RaftConfiguration> {
    |obj: Option<&RaftConfiguration>| {
        if let Some(config) = obj {
            return config.data.config.servers.iter().all(|s| s.voter);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use secrecy::Secret;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::{
        is_seal_status_initialized, raft_configuration_all_voters, raft_configuration_any_leader,
        GetRaftConfiguration, GetSealStatus, HttpForwarderService, RaftConfiguration,
    };

    fn minimal_seal_status() -> serde_json::Value {
        serde_json::json!({
            "type": "shamir",
            "initialized": false,
            "sealed": true,
            "t": 2,
            "n": 3,
            "progress": 0,
            "nonce": "",
            "version": "1.13.0",
            "build_date": "2023-03-01T14:58:13Z",
            "migration": false,
            "recovery_seal": false,
            "storage_type": "raft"
        })
    }

    fn uninitialized_seal_status() -> serde_json::Value {
        serde_json::json!({
            "type": "shamir",
            "initialized": false,
            "sealed": true,
            "t": 2,
            "n": 3,
            "progress": 0,
            "nonce": "",
            "version": "1.13.0",
            "build_date": "2023-03-01T14:58:13Z",
            "migration": false,
            "recovery_seal": false,
            "storage_type": "raft",
            "ha_enabled": true,
            "active_time": "0001-01-01T00:00:00Z"
        })
    }

    fn initialized_seal_status() -> serde_json::Value {
        serde_json::json!({
            "type": "shamir",
            "initialized": true,
            "sealed": false,
            "t": 2,
            "n": 3,
            "progress": 0,
            "nonce": "",
            "version": "1.13.0",
            "build_date": "2023-03-01T14:58:13Z",
            "migration": false,
            "cluster_name": "vault-cluster-211d673a",
            "cluster_id": "b7b7f5e2-803a-2484-df4a-870c6b15f22f",
            "recovery_seal": false,
            "storage_type": "raft",
            "ha_enabled": true,
            "active_time": "0001-01-01T00:00:00Z",
            "leader_address": "http://10.42.2.25:8200",
            "leader_cluster_address": "https://vault-0.vault-internal:8201",
            "raft_committed_index": 40,
            "raft_applied_index": 40
        })
    }

    async fn mock(response: serde_json::Value) -> MockServer {
        let mock_server = MockServer::start().await;

        Mock::given(method(http::Method::GET))
            .and(path("/v1/sys/seal-status"))
            .and(header("X-Vault-Request", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&mock_server)
            .await;

        mock_server
    }

    #[tokio::test]
    async fn getting_seal_status_works_if_minimal() {
        let mock_server = mock(minimal_seal_status()).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let status = client.seal_status().await.unwrap();

        assert_eq!(status.type_, "shamir");
        assert!(!status.initialized);
        assert!(status.sealed);
        assert_eq!(status.t, 2);
        assert_eq!(status.n, 3);
        assert_eq!(status.progress, 0);
        assert_eq!(status.nonce, "");
        assert_eq!(status.version, "1.13.0");
        assert_eq!(status.build_date, "2023-03-01T14:58:13Z");
        assert!(!status.migration);
        assert!(!status.recovery_seal);
        assert_eq!(status.storage_type, "raft");
        assert_eq!(status.ha_enabled, None);
        assert_eq!(status.cluster_name, None);
        assert_eq!(status.cluster_id, None);
        assert_eq!(status.active_time, None);
        assert_eq!(status.leader_address, None);
        assert_eq!(status.leader_cluster_address, None);
        assert_eq!(status.raft_committed_index, None);
        assert_eq!(status.raft_applied_index, None);
    }

    #[tokio::test]
    async fn getting_seal_status_works_if_uninitialized() {
        let mock_server = mock(uninitialized_seal_status()).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let status = client.seal_status().await.unwrap();

        assert_eq!(status.type_, "shamir");
        assert!(!status.initialized);
        assert!(status.sealed);
        assert_eq!(status.t, 2);
        assert_eq!(status.n, 3);
        assert_eq!(status.progress, 0);
        assert_eq!(status.nonce, "");
        assert_eq!(status.version, "1.13.0");
        assert_eq!(status.build_date, "2023-03-01T14:58:13Z");
        assert!(!status.migration);
        assert!(!status.recovery_seal);
        assert_eq!(status.storage_type, "raft");
        assert!(status.ha_enabled.unwrap());
        assert_eq!(status.cluster_name, None);
        assert_eq!(status.cluster_id, None);
        assert_eq!(status.active_time.unwrap(), "0001-01-01T00:00:00Z");
        assert_eq!(status.leader_address, None);
        assert_eq!(status.leader_cluster_address, None);
        assert_eq!(status.raft_committed_index, None);
        assert_eq!(status.raft_applied_index, None);
    }

    #[tokio::test]
    async fn getting_seal_status_works_if_initialized() {
        let mock_server = mock(initialized_seal_status()).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let status = client.seal_status().await.unwrap();

        assert_eq!(status.type_, "shamir");
        assert!(status.initialized);
        assert!(!status.sealed);
        assert_eq!(status.t, 2);
        assert_eq!(status.n, 3);
        assert_eq!(status.progress, 0);
        assert_eq!(status.nonce, "");
        assert_eq!(status.version, "1.13.0");
        assert_eq!(status.build_date, "2023-03-01T14:58:13Z");
        assert!(!status.migration);
        assert!(!status.recovery_seal);
        assert_eq!(status.storage_type, "raft");
        assert!(status.ha_enabled.unwrap());
        assert_eq!(status.cluster_name.unwrap(), "vault-cluster-211d673a");
        assert_eq!(
            status.cluster_id.unwrap(),
            "b7b7f5e2-803a-2484-df4a-870c6b15f22f"
        );
        assert_eq!(status.active_time.unwrap(), "0001-01-01T00:00:00Z");
        assert_eq!(status.leader_address.unwrap(), "http://10.42.2.25:8200");
        assert_eq!(
            status.leader_cluster_address.unwrap(),
            "https://vault-0.vault-internal:8201"
        );
        assert_eq!(status.raft_committed_index.unwrap(), 40);
        assert_eq!(status.raft_applied_index.unwrap(), 40);
    }

    #[tokio::test]
    async fn waiting_for_seal_status_works() {
        let mock_server = mock(initialized_seal_status()).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let status = client
            .await_seal_status(is_seal_status_initialized())
            .await
            .unwrap()
            .unwrap();

        assert!(status.initialized);
    }

    fn raft_configuration() -> serde_json::Value {
        serde_json::json!({
            "request_id": "7f6fc909-bb7f-e48c-d850-0ad8a22cb434",
            "lease_id": "",
            "renewable": false,
            "lease_duration": 0,
            "data": {
                "config": {
                    "servers": [
                        {
                            "node_id": "147c957f-5718-07b6-424e-5522efcfbc9e",
                            "address": "vault-0.vault-internal:8201",
                            "leader": true,
                            "protocol_version": "3",
                            "voter": true
                        },
                        {
                            "node_id": "04ffa935-e1c2-e891-a9e9-426bf1a6c93d",
                            "address": "vault-1.vault-internal:8201",
                            "leader": false,
                            "protocol_version": "3",
                            "voter": true
                        },
                        {
                            "node_id": "124bef00-64ec-59de-1366-7050edfb5c49",
                            "address": "vault-2.vault-internal:8201",
                            "leader": false,
                            "protocol_version": "3",
                            "voter": true
                        }
                    ],
                    "index": 0
                }
            },
            "wrap_info": null,
            "warnings": null,
            "auth": null
        })
    }

    fn raft_configuration_no_leader() -> serde_json::Value {
        let mut rc = serde_json::from_value::<RaftConfiguration>(raft_configuration()).unwrap();
        rc.data.config.servers[0].leader = false;
        serde_json::to_value(rc).unwrap()
    }

    fn raft_configuration_single_non_voter() -> serde_json::Value {
        let mut rc = serde_json::from_value::<RaftConfiguration>(raft_configuration()).unwrap();
        rc.data.config.servers[2].voter = false;
        serde_json::to_value(rc).unwrap()
    }

    fn raft_configuration_no_voter() -> serde_json::Value {
        let mut rc = serde_json::from_value::<RaftConfiguration>(raft_configuration()).unwrap();
        rc.data.config.servers[0].leader = false;
        rc.data
            .config
            .servers
            .iter_mut()
            .for_each(|s| s.voter = false);
        serde_json::to_value(rc).unwrap()
    }

    async fn mock_raft_configuration(response: &[serde_json::Value]) -> MockServer {
        let mock_server = MockServer::start().await;

        for r in response {
            Mock::given(method(http::Method::GET))
                .and(path("/v1/sys/storage/raft/configuration"))
                .and(header("X-Vault-Request", "true"))
                .and(header("X-Vault-Token", "abc"))
                .respond_with(ResponseTemplate::new(200).set_body_json(r))
                .up_to_n_times(1)
                .expect(1)
                .mount(&mock_server)
                .await;
        }

        mock_server
    }

    #[tokio::test]
    async fn getting_raft_configuration_works() {
        let mock_server = mock_raft_configuration(&[raft_configuration()]).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let config = client
            .raft_configuration(Secret::from_str("abc").unwrap())
            .await
            .unwrap();

        assert_eq!(config.request_id, "7f6fc909-bb7f-e48c-d850-0ad8a22cb434");
        assert_eq!(config.lease_id, "");
        assert!(!config.renewable);
        assert_eq!(config.lease_duration, 0);
        assert_eq!(config.data.config.index, 0);
        assert_eq!(config.data.config.servers.len(), 3);
        assert_eq!(
            config.data.config.servers[0].node_id,
            "147c957f-5718-07b6-424e-5522efcfbc9e"
        );
        assert_eq!(
            config.data.config.servers[0].address,
            "vault-0.vault-internal:8201"
        );
        assert!(config.data.config.servers[0].leader);
        assert_eq!(config.data.config.servers[0].protocol_version, "3");
        assert!(config.data.config.servers[0].voter);
        assert_eq!(
            config.data.config.servers[1].node_id,
            "04ffa935-e1c2-e891-a9e9-426bf1a6c93d"
        );
        assert_eq!(
            config.data.config.servers[1].address,
            "vault-1.vault-internal:8201"
        );
        assert!(!config.data.config.servers[1].leader);
        assert_eq!(config.data.config.servers[1].protocol_version, "3");
        assert!(config.data.config.servers[1].voter);
        assert_eq!(
            config.data.config.servers[2].node_id,
            "124bef00-64ec-59de-1366-7050edfb5c49"
        );
        assert_eq!(
            config.data.config.servers[2].address,
            "vault-2.vault-internal:8201"
        );
        assert!(!config.data.config.servers[2].leader);
        assert_eq!(config.data.config.servers[2].protocol_version, "3");
        assert!(config.data.config.servers[2].voter);

        assert_eq!(config.wrap_info, None);
        assert_eq!(config.warnings, None);
        assert_eq!(config.auth, None);
    }

    #[tokio::test]
    async fn waiting_for_raft_configuration_works() {
        let mock_server = mock_raft_configuration(&[raft_configuration()]).await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let config = client
            .await_raft_configuration(
                Secret::from_str("abc").unwrap(),
                raft_configuration_any_leader(),
            )
            .await
            .unwrap()
            .unwrap();

        assert!(config.data.config.servers[0].leader);
    }

    #[tokio::test]
    async fn waiting_for_raft_configuration_having_leader_works() {
        let mock_server = mock_raft_configuration(&[
            raft_configuration_no_leader(),
            raft_configuration_no_leader(),
            raft_configuration(),
        ])
        .await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let config = client
            .await_raft_configuration(
                Secret::from_str("abc").unwrap(),
                raft_configuration_any_leader(),
            )
            .await
            .unwrap()
            .unwrap();

        assert!(config.data.config.servers[0].leader);
    }

    #[tokio::test]
    async fn waiting_for_raft_configuration_having_all_voters_works() {
        let mock_server = mock_raft_configuration(&[
            raft_configuration_no_voter(),
            raft_configuration_single_non_voter(),
            raft_configuration(),
        ])
        .await;

        let mut client = HttpForwarderService::http(
            tokio::net::TcpStream::connect(mock_server.uri().strip_prefix("http://").unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        let config = client
            .await_raft_configuration(
                Secret::from_str("abc").unwrap(),
                raft_configuration_all_voters(),
            )
            .await
            .unwrap()
            .unwrap();

        assert!(config.data.config.servers.iter().all(|s| s.voter));
    }
}
