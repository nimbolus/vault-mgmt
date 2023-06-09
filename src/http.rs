use std::sync::Arc;

use http::Response;
use hyper::{Body, Request};
use hyper_rustls::ConfigBuilderExt;
use secrecy::{ExposeSecret, Secret};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::*;

/// Send HTTP requests
#[async_trait::async_trait]
pub trait HttpRequest {
    /// Send an HTTP request and return the response
    async fn send_request(
        &mut self,
        req: Request<hyper::body::Body>,
    ) -> hyper::Result<Response<Body>>;

    /// Wait until the connection is ready to send requests
    async fn ready(&mut self) -> anyhow::Result<()>;
}

/// Forward HTTP requests over a connection stream
pub struct HttpForwarderService {
    sender: hyper::client::conn::http1::SendRequest<Body>,
}

impl HttpForwarderService {
    /// Forward HTTP requests over a connection stream
    pub async fn http<T>(stream: T) -> anyhow::Result<HttpForwarderService>
    where
        T: AsyncRead + AsyncWrite + Unpin + Sync + Send + 'static,
    {
        let (sender, connection) = hyper::client::conn::http1::handshake(stream).await?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                warn!("Error in connection: {}", e);
            }
        });

        Ok(Self { sender })
    }

    /// Wrap the connection stream in TLS and forward HTTP requests over it
    /// The domain is used to verify the TLS certificate
    /// The native root certificates are used to verify the TLS certificate
    ///
    /// TODO: allow customizing the TLS configuration
    pub async fn https<T>(domain: &str, stream: T) -> anyhow::Result<HttpForwarderService>
    where
        T: AsyncRead + AsyncWrite + Unpin + Sync + Send + 'static,
    {
        let tls = tokio_rustls::rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_native_roots()
            .with_no_client_auth();

        let tls_stream = tokio_rustls::TlsConnector::from(Arc::new(tls))
            .connect(tokio_rustls::rustls::ServerName::try_from(domain)?, stream)
            .await?;

        HttpForwarderService::http(tls_stream).await
    }
}

#[async_trait::async_trait]
impl HttpRequest for HttpForwarderService {
    async fn send_request(
        &mut self,
        req: Request<hyper::body::Body>,
    ) -> hyper::Result<Response<Body>> {
        let (parts, body) = self.sender.send_request(req).await?.into_parts();
        let body = hyper::body::to_bytes(body).await?;
        Ok(Response::from_parts(parts, body.into()))
    }

    async fn ready(&mut self) -> anyhow::Result<()> {
        self.sender
            .ready()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}

pub const VAULT_PORT: u16 = 8200;

pub(crate) fn vault_request() -> http::request::Builder {
    hyper::Request::builder()
        .header("Host", "127.0.0.1")
        .header("x-Vault-Request", "true")
}

pub(crate) fn vault_request_with_token(token: Secret<String>) -> http::request::Builder {
    vault_request().header("X-Vault-Token", token.expose_secret())
}

const SEAL_STATUS_URL: &str = "/v1/sys/seal-status";
pub(crate) fn seal_status_request(body: hyper::Body) -> http::Result<Request<Body>> {
    vault_request()
        .uri(SEAL_STATUS_URL)
        .method(hyper::Method::GET)
        .body(body)
}

const UNSEAL_URL: &str = "/v1/sys/unseal";
pub(crate) fn unseal_request(body: hyper::Body) -> http::Result<Request<Body>> {
    vault_request()
        .uri(UNSEAL_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

pub(crate) fn get_unseal_keys_request(
    path: &str,
    token: Secret<String>,
) -> http::Result<Request<Body>> {
    vault_request_with_token(token)
        .uri(path)
        .method(hyper::Method::GET)
        .body(hyper::Body::empty())
}

const INIT_URL: &str = "/v1/sys/init";
pub(crate) fn init_request(body: hyper::Body) -> http::Result<Request<Body>> {
    vault_request()
        .uri(INIT_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

const RAFT_JOIN_URL: &str = "/v1/sys/storage/raft/join";
pub(crate) fn raft_join_request(body: hyper::Body) -> http::Result<Request<Body>> {
    vault_request()
        .uri(RAFT_JOIN_URL)
        .method(hyper::Method::POST)
        .body(body)
}

const RAFT_CONFIGURATION_URL: &str = "/v1/sys/storage/raft/configuration";
pub(crate) fn raft_configuration_request(
    token: Secret<String>,
    body: hyper::Body,
) -> http::Result<Request<Body>> {
    vault_request_with_token(token)
        .uri(RAFT_CONFIGURATION_URL)
        .method(hyper::Method::GET)
        .body(body)
}

const STEP_DOWN_URL: &str = "/v1/sys/step-down";
pub(crate) fn step_down_request(
    token: Secret<String>,
    body: hyper::Body,
) -> http::Result<Request<Body>> {
    vault_request_with_token(token)
        .uri(STEP_DOWN_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};

    use crate::http::{HttpForwarderService, HttpRequest};

    #[tokio::test]
    async fn http_forward_works() {
        let mock_server = MockServer::start().await;

        Mock::given(any())
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

        let http_req = hyper::Request::builder()
            .uri("/")
            .method(hyper::Method::GET)
            .body(hyper::Body::from(""))
            .unwrap();

        let (parts, _) = client.send_request(http_req).await.unwrap().into_parts();

        assert!(parts.status.is_success());
    }

    // TODO: do not use remote host for testing
    // #[tokio::test]
    // async fn https_forward_works() {
    //     const DOMAIN: &str = "google.com";

    //     let mut pf = HttpForwarderService::https(
    //         DOMAIN,
    //         tokio::net::TcpStream::connect(&format!("{}:443", DOMAIN))
    //             .await
    //             .unwrap(),
    //     )
    //     .await
    //     .unwrap();

    //     let http_req = hyper::Request::builder()
    //         .uri("/")
    //         .header("Host", DOMAIN)
    //         .method(hyper::Method::GET)
    //         .body(hyper::Body::from(""))
    //         .unwrap();

    //     let (parts, _) = pf.send_request(http_req).await.unwrap().into_parts();

    //     assert!(parts.status.is_success() || parts.status.is_redirection());
    // }
}
