use std::{convert::Infallible, sync::Arc};

use http::Response;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::{
    body::{Body, Bytes},
    Request,
};
use hyper_util::rt::TokioIo;
use rustls::crypto::ring;
use secrecy::{ExposeSecret, Secret};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::{pki_types, RootCertStore};
use tracing::*;

pub type BytesBody = BoxBody<Bytes, Infallible>;

/// Send HTTP requests
#[async_trait::async_trait]
pub trait HttpRequest<B>
where
    B: Body,
{
    /// Send an HTTP request and return the response
    async fn send_request(&mut self, req: Request<B>) -> hyper::Result<Response<Bytes>>;

    /// Wait until the connection is ready to send requests
    async fn ready(&mut self) -> anyhow::Result<()>;
}

/// Forward HTTP requests over a connection stream
pub struct HttpForwarderService<B>
where
    B: Body,
{
    sender: hyper::client::conn::http1::SendRequest<B>,
}

impl<B> HttpForwarderService<B>
where
    B: Body<Data = Bytes, Error = Infallible> + Send + 'static,
{
    /// Forward HTTP requests over a connection stream
    pub async fn http<T>(stream: T) -> anyhow::Result<HttpForwarderService<B>>
    where
        T: AsyncRead + AsyncWrite + Unpin + Sync + Send + 'static,
    {
        let io = TokioIo::new(stream);

        let (sender, connection) = hyper::client::conn::http1::handshake(io).await?;

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
    pub async fn https<T>(domain: &str, stream: T) -> anyhow::Result<HttpForwarderService<B>>
    where
        T: AsyncRead + AsyncWrite + Unpin + Sync + Send + 'static,
    {
        let tls_stream = setup_tls(domain, stream).await?;

        HttpForwarderService::http(tls_stream).await
    }
}

#[async_trait::async_trait]
impl<B> HttpRequest<B> for HttpForwarderService<B>
where
    B: Body<Data = Bytes, Error = Infallible> + Send + 'static,
{
    async fn send_request(&mut self, req: Request<B>) -> hyper::Result<Response<Bytes>> {
        let (parts, body) = self.sender.send_request(req).await?.into_parts();
        let body = body.boxed().collect().await?.to_bytes();
        Ok(Response::from_parts(parts, body))
    }

    async fn ready(&mut self) -> anyhow::Result<()> {
        self.sender
            .ready()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}

pub(crate) async fn setup_tls<T>(
    domain: &str,
    stream: T,
) -> anyhow::Result<tokio_rustls::client::TlsStream<T>>
where
    T: AsyncRead + AsyncWrite + Unpin + Sync + Send + 'static,
{
    let mut root_cert_store = RootCertStore::empty();

    for cert in rustls_native_certs::load_native_certs()
        .map_err(|e| anyhow::anyhow!("could not load platform certs: {}", e))?
    {
        root_cert_store.add(cert).unwrap();
    }

    let tls = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
        ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_cert_store)
    .with_no_client_auth();

    let tls_stream = tokio_rustls::TlsConnector::from(Arc::new(tls))
        .connect(pki_types::ServerName::try_from(domain)?.to_owned(), stream)
        .await?;

    Ok(tls_stream)
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
pub(crate) fn seal_status_request(body: BytesBody) -> http::Result<Request<BytesBody>> {
    vault_request()
        .uri(SEAL_STATUS_URL)
        .method(hyper::Method::GET)
        .body(body)
}

const UNSEAL_URL: &str = "/v1/sys/unseal";
pub(crate) fn unseal_request(body: BytesBody) -> http::Result<Request<BytesBody>> {
    vault_request()
        .uri(UNSEAL_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

pub(crate) fn get_unseal_keys_request(
    path: &str,
    token: Secret<String>,
) -> http::Result<Request<BytesBody>> {
    vault_request_with_token(token)
        .uri(path)
        .method(hyper::Method::GET)
        .body(Empty::<Bytes>::new().boxed())
}

const INIT_URL: &str = "/v1/sys/init";
pub(crate) fn init_request(body: BytesBody) -> http::Result<Request<BytesBody>> {
    vault_request()
        .uri(INIT_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

const RAFT_JOIN_URL: &str = "/v1/sys/storage/raft/join";
pub(crate) fn raft_join_request(body: BytesBody) -> http::Result<Request<BytesBody>> {
    vault_request()
        .uri(RAFT_JOIN_URL)
        .method(hyper::Method::POST)
        .body(body)
}

const RAFT_CONFIGURATION_URL: &str = "/v1/sys/storage/raft/configuration";
pub(crate) fn raft_configuration_request(
    token: Secret<String>,
    body: BytesBody,
) -> http::Result<Request<BytesBody>> {
    vault_request_with_token(token)
        .uri(RAFT_CONFIGURATION_URL)
        .method(hyper::Method::GET)
        .body(body)
}

const STEP_DOWN_URL: &str = "/v1/sys/step-down";
pub(crate) fn step_down_request(
    token: Secret<String>,
    body: BytesBody,
) -> http::Result<Request<BytesBody>> {
    vault_request_with_token(token)
        .uri(STEP_DOWN_URL)
        .method(hyper::Method::PUT)
        .body(body)
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use http_body_util::Empty;
    use hyper::body::Bytes;
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
            .body(Empty::<Bytes>::new())
            .unwrap();

        let (parts, _) = client.send_request(http_req).await.unwrap().into_parts();

        assert!(parts.status.is_success());
    }

    // TODO: do not use remote host for testing
    #[ignore = "connecting to google.com"]
    #[tokio::test]
    async fn https_forward_works() {
        const DOMAIN: &str = "google.com";

        let stream = tokio::net::TcpStream::connect(&format!("{}:443", DOMAIN))
            .await
            .unwrap();

        let mut pf = HttpForwarderService::https(DOMAIN, stream).await.unwrap();

        let http_req = hyper::Request::builder()
            .uri("/")
            .header("Host", DOMAIN)
            .method(hyper::Method::GET)
            .body(Empty::<Bytes>::new())
            .unwrap();

        let (parts, _) = pf.send_request(http_req).await.unwrap().into_parts();

        assert!(parts.status.is_success() || parts.status.is_redirection());
    }
}
