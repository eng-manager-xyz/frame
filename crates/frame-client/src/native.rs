use reqwest::{
    Client,
    header::{CONTENT_TYPE, LOCATION},
    redirect::Policy,
};

use crate::{
    BoxFuture, ClientError, ClientErrorCode, HttpMethod, Transport, TransportPolicy,
    TransportRequest, TransportResponse,
};

#[derive(Clone)]
pub struct NativeTransport {
    client: Client,
}

impl NativeTransport {
    pub fn new() -> Result<Self, ClientError> {
        Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent("frame-client/0.1")
            .build()
            .map(|client| Self { client })
            .map_err(|_| ClientError::new(ClientErrorCode::TransportUnavailable))
    }
}

impl Transport for NativeTransport {
    fn send<'a>(
        &'a self,
        request: TransportRequest,
        policy: &'a TransportPolicy,
    ) -> BoxFuture<'a, Result<TransportResponse, ClientError>> {
        Box::pin(async move {
            let builder = match request.method() {
                HttpMethod::Get => self.client.get(request.url()),
                HttpMethod::Head => self.client.head(request.url()),
            }
            .timeout(policy.deadline())
            .header("accept", "application/json");

            let mut response = builder.send().await.map_err(map_reqwest_error)?;
            let status = response.status().as_u16();
            if response.headers().contains_key(LOCATION) || (300..400).contains(&status) {
                return Err(ClientError::new(ClientErrorCode::RedirectRejected));
            }
            if response
                .content_length()
                .is_some_and(|length| length > policy.max_response_bytes() as u64)
            {
                return Err(ClientError::new(ClientErrorCode::ResponseTooLarge));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
                if body.len().saturating_add(chunk.len()) > policy.max_response_bytes() {
                    return Err(ClientError::new(ClientErrorCode::ResponseTooLarge));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(TransportResponse {
                status,
                content_type,
                body,
            })
        })
    }
}

fn map_reqwest_error(error: reqwest::Error) -> ClientError {
    if error.is_timeout() {
        ClientError::new(ClientErrorCode::DeadlineExceeded)
    } else {
        ClientError::new(ClientErrorCode::TransportUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_transport_builds_without_ambient_proxy_or_redirect_policy() {
        NativeTransport::new().expect("native transport");
    }
}
