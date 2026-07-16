use std::{fmt, future::Future, pin::Pin, time::Duration};

use serde::de::DeserializeOwned;

use crate::{ClientError, ClientErrorCode, FrameOrigin, Health, PublicShareSummary};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
}

impl HttpMethod {
    #[must_use]
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::Get | Self::Head)
    }
}

#[derive(Clone)]
pub struct TransportRequest {
    method: HttpMethod,
    url: String,
}

impl TransportRequest {
    pub fn api_get(origin: &FrameOrigin, resource: &str) -> Result<Self, ClientError> {
        Ok(Self {
            method: HttpMethod::Get,
            url: origin.api_v1_url(resource)?,
        })
    }

    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for TransportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportRequest")
            .field("method", &self.method)
            .field("url", &"<redacted-frame-url>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TransportResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl fmt::Debug for TransportResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportResponse")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field(
                "body",
                &format_args!("<redacted:{} bytes>", self.body.len()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportPolicy {
    deadline: Duration,
    max_response_bytes: usize,
    max_attempts: u8,
}

impl TransportPolicy {
    pub fn new(
        deadline: Duration,
        max_response_bytes: usize,
        max_attempts: u8,
    ) -> Result<Self, ClientError> {
        if deadline.is_zero()
            || deadline > Duration::from_secs(30)
            || !(1_024..=1024 * 1024).contains(&max_response_bytes)
            || !(1..=3).contains(&max_attempts)
        {
            return Err(ClientError::new(ClientErrorCode::InvalidContract));
        }
        Ok(Self {
            deadline,
            max_response_bytes,
            max_attempts,
        })
    }

    #[must_use]
    pub const fn deadline(&self) -> Duration {
        self.deadline
    }

    #[must_use]
    pub const fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }
}

impl Default for TransportPolicy {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(3),
            max_response_bytes: 64 * 1024,
            max_attempts: 2,
        }
    }
}

/// Consumer-supplied transport. Implementations must enforce the supplied
/// deadline and byte limit before returning. Dropping the returned future must
/// cancel in-flight work; redirects must not be followed implicitly.
pub trait Transport {
    fn send<'a>(
        &'a self,
        request: TransportRequest,
        policy: &'a TransportPolicy,
    ) -> BoxFuture<'a, Result<TransportResponse, ClientError>>;
}

pub struct FrameClient<T> {
    origin: FrameOrigin,
    transport: T,
    policy: TransportPolicy,
}

impl<T> FrameClient<T> {
    #[must_use]
    pub fn new(origin: FrameOrigin, transport: T) -> Self {
        Self {
            origin,
            transport,
            policy: TransportPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_policy(mut self, policy: TransportPolicy) -> Self {
        self.policy = policy;
        self
    }

    #[must_use]
    pub fn origin(&self) -> &FrameOrigin {
        &self.origin
    }
}

impl<T: Transport> FrameClient<T> {
    pub async fn health(&self) -> Result<Health, ClientError> {
        let health: Health = self.get_json("health").await?;
        health.validate()?;
        Ok(health)
    }

    pub async fn public_share(&self, identifier: &str) -> Result<PublicShareSummary, ClientError> {
        validate_identifier(identifier)?;
        let summary: PublicShareSummary = self
            .get_json(&format!("public/shares/{identifier}"))
            .await?;
        summary.validate(&self.origin)?;
        Ok(summary)
    }

    async fn get_json<R: DeserializeOwned>(&self, resource: &str) -> Result<R, ClientError> {
        let request = TransportRequest::api_get(&self.origin, resource)?;
        let mut attempt = 0_u8;
        loop {
            attempt += 1;
            match self.transport.send(request.clone(), &self.policy).await {
                Ok(response) => return decode_json(response, &self.policy),
                Err(error)
                    if request.method().is_idempotent()
                        && error.retryable()
                        && attempt < self.policy.max_attempts() => {}
                Err(error) => return Err(error),
            }
        }
    }
}

fn decode_json<R: DeserializeOwned>(
    response: TransportResponse,
    policy: &TransportPolicy,
) -> Result<R, ClientError> {
    if (300..400).contains(&response.status) {
        return Err(ClientError::new(ClientErrorCode::RedirectRejected));
    }
    if !(200..300).contains(&response.status) {
        return Err(ClientError::new(ClientErrorCode::ApiRejected));
    }
    if response.body.len() > policy.max_response_bytes() {
        return Err(ClientError::new(ClientErrorCode::ResponseTooLarge));
    }
    let content_type = response
        .content_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type != Some("application/json") {
        return Err(ClientError::new(ClientErrorCode::UnsupportedContentType));
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| ClientError::new(ClientErrorCode::MalformedResponse))
}

fn validate_identifier(value: &str) -> Result<(), ClientError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ClientError::new(ClientErrorCode::InvalidIdentifier));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::Mutex,
        task::{Context, Poll, Waker},
    };

    use super::*;

    const HEALTH: &str = include_str!("../../../fixtures/frame-api/v1/health.ok.json");

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<TransportResponse, ClientError>>>,
        attempts: Mutex<usize>,
    }

    impl FakeTransport {
        fn new(
            responses: impl IntoIterator<Item = Result<TransportResponse, ClientError>>,
        ) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                attempts: Mutex::new(0),
            }
        }
    }

    impl Transport for FakeTransport {
        fn send<'a>(
            &'a self,
            _request: TransportRequest,
            _policy: &'a TransportPolicy,
        ) -> BoxFuture<'a, Result<TransportResponse, ClientError>> {
            Box::pin(async move {
                *self.attempts.lock().expect("attempt lock") += 1;
                self.responses
                    .lock()
                    .expect("response lock")
                    .pop_front()
                    .expect("fake response")
            })
        }
    }

    fn run<F: Future>(future: F) -> F::Output {
        let mut context = Context::from_waker(Waker::noop());
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn response(body: &str) -> TransportResponse {
        TransportResponse {
            status: 200,
            content_type: Some("application/json; charset=utf-8".into()),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn client_decodes_and_validates_health() {
        let transport = FakeTransport::new([Ok(response(HEALTH))]);
        let client = FrameClient::new(
            FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin"),
            transport,
        );
        let health = run(client.health()).expect("health response");
        assert_eq!(health.service, "frame");
    }

    #[test]
    fn idempotent_get_retries_only_bounded_retryable_failures() {
        let transport = FakeTransport::new([
            Err(ClientError::new(ClientErrorCode::DeadlineExceeded)),
            Ok(response(HEALTH)),
        ]);
        let client = FrameClient::new(
            FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin"),
            transport,
        );
        run(client.health()).expect("retried health");
        assert_eq!(*client.transport.attempts.lock().expect("attempt lock"), 2);
    }

    #[test]
    fn content_type_size_redirect_and_identifier_fail_closed() {
        let policy = TransportPolicy::default();
        let oversized = TransportResponse {
            status: 200,
            content_type: Some("application/json".into()),
            body: vec![b'x'; policy.max_response_bytes() + 1],
        };
        assert_eq!(
            decode_json::<Health>(oversized, &policy)
                .expect_err("oversize")
                .code(),
            ClientErrorCode::ResponseTooLarge
        );
        let redirect = TransportResponse {
            status: 302,
            content_type: None,
            body: Vec::new(),
        };
        assert_eq!(
            decode_json::<Health>(redirect, &policy)
                .expect_err("redirect")
                .code(),
            ClientErrorCode::RedirectRejected
        );
        assert!(validate_identifier("../private").is_err());
    }

    #[test]
    fn request_and_response_debug_redact_payloads() {
        let origin = FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin");
        let request = TransportRequest::api_get(&origin, "health").expect("request");
        assert!(!format!("{request:?}").contains("frame.engmanager.xyz"));
        let response = response("frame-secret-response-body");
        assert!(!format!("{response:?}").contains("frame-secret-response-body"));
    }
}
