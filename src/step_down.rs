use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use secrecy::Secret;

use crate::{step_down_request, BytesBody, HttpRequest};

/// Step down vault pod from active to standby
#[async_trait::async_trait]
pub trait StepDown {
    /// Step down vault pod from active to standby
    async fn step_down(&mut self, token: Secret<String>) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl<T> StepDown for T
where
    T: HttpRequest<BytesBody> + Send + Sync + 'static,
{
    async fn step_down(&mut self, token: Secret<String>) -> anyhow::Result<()> {
        let http_req = step_down_request(token, Empty::<Bytes>::new().boxed())?;

        let (parts, body) = self.send_request(http_req).await?.into_parts();

        let body = String::from_utf8(body.to_vec())?;

        if parts.status != hyper::StatusCode::NO_CONTENT {
            return Err(anyhow::anyhow!("stepping-down: {}", body));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use http::{Method, StatusCode};
    use secrecy::Secret;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::{HttpForwarderService, StepDown};

    #[tokio::test]
    async fn stepdown_calls_api() {
        let mock_server = MockServer::start().await;

        Mock::given(method(Method::PUT))
            .and(path("/v1/sys/step-down"))
            .and(header("X-Vault-Request", "true"))
            .and(header("X-Vault-Token", "abc"))
            .respond_with(ResponseTemplate::new(StatusCode::NO_CONTENT))
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

        let outcome = client.step_down(Secret::from_str("abc").unwrap()).await;

        assert!(outcome.is_ok());
    }
}
