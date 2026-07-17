//! Authenticated desktop adapter for the Instant finalize boundary.

use std::{fmt, time::Duration};

use frame_authenticated_client::{
    InstantFinalizeReceiptV1, InstantFinalizeRequestV1, InstantFinalizeStateV1,
};
use frame_client::FrameOrigin;
use frame_media::{
    InstantFinalizeReceipt, InstantFinalizeRequest, InstantOperationId, InstantPublicationId,
};
use reqwest::{
    Client,
    header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue, LOCATION},
    redirect::Policy,
};
use thiserror::Error;

const FINALIZE_DEADLINE: Duration = Duration::from_secs(15);
const MAX_FINALIZE_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstantFinalizeDispatchOutcome {
    Pending,
    Published(Box<InstantFinalizeReceipt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DesktopInstantFinalizeError {
    #[error("the finalize request context is invalid")]
    InvalidContract,
    #[error("the bearer credential is invalid")]
    InvalidCredential,
    #[error("the finalize transport is unavailable")]
    TransportUnavailable,
    #[error("the finalize request exceeded its deadline")]
    DeadlineExceeded,
    #[error("a finalize redirect was rejected")]
    RedirectRejected,
    #[error("the finalize response exceeded its byte limit")]
    ResponseTooLarge,
    #[error("the finalize response content type is unsupported")]
    UnsupportedContentType,
    #[error("the finalize API rejected the request with status {0}")]
    ApiRejected(u16),
    #[error("the finalize response was malformed")]
    MalformedResponse,
    #[error("the finalize response violated the request contract")]
    InvalidResponse,
}

/// Concrete native caller available to the desktop command layer. Redirects
/// and ambient proxies are disabled so authorization cannot be forwarded
/// outside the validated Frame origin.
///
/// The current Tauri command surface does not invoke this adapter yet. Keeping
/// that wiring boundary explicit prevents the loopback contract tests below
/// from being mistaken for an end-to-end production finalize journey.
#[derive(Clone)]
pub struct DesktopInstantFinalizeClient {
    origin: FrameOrigin,
    client: Client,
}

impl fmt::Debug for DesktopInstantFinalizeClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopInstantFinalizeClient")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl DesktopInstantFinalizeClient {
    pub fn new(origin: FrameOrigin) -> Result<Self, DesktopInstantFinalizeError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent("frame-desktop/0.1")
            .build()
            .map_err(|_| DesktopInstantFinalizeError::TransportUnavailable)?;
        Ok(Self { origin, client })
    }

    /// Reconciles one sealed native finalize request. The operation ID is also
    /// the retry identity: retries must reuse it, while a new logical attempt
    /// must provide a new CSPRNG identity.
    pub async fn dispatch(
        &self,
        tenant_id: &str,
        video_id: &str,
        bearer_token: &str,
        operation_id: InstantOperationId,
        request: &InstantFinalizeRequest,
    ) -> Result<InstantFinalizeDispatchOutcome, DesktopInstantFinalizeError> {
        if !valid_bearer_token(bearer_token) {
            return Err(DesktopInstantFinalizeError::InvalidCredential);
        }
        let wire = instant_finalize_wire_request(tenant_id, video_id, operation_id, request)?;
        let body =
            serde_json::to_vec(&wire).map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let endpoint = self
            .origin
            .api_v1_url(&format!("instant-recordings/{}/finalize", wire.session_id))
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let authorization = HeaderValue::from_str(&format!("Bearer {bearer_token}"))
            .map_err(|_| DesktopInstantFinalizeError::InvalidCredential)?;
        let tenant = HeaderValue::from_str(&wire.tenant_id)
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let operation_key = format!("instant-finalize:{}", wire.operation_id);
        let idempotency = HeaderValue::from_str(&operation_key)
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;

        let mut response = self
            .client
            .post(endpoint)
            .timeout(FINALIZE_DEADLINE)
            .header(AUTHORIZATION, authorization)
            .header("x-frame-tenant-id", tenant)
            .header("idempotency-key", idempotency)
            .header(CONTENT_TYPE, "application/json")
            .header("accept", "application/json")
            .body(body)
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status().as_u16();
        if response.headers().contains_key(LOCATION) || (300..400).contains(&status) {
            return Err(DesktopInstantFinalizeError::RedirectRejected);
        }
        if !matches!(status, 200 | 202) {
            return Err(DesktopInstantFinalizeError::ApiRejected(status));
        }
        validate_response_headers(response.headers())?;

        let mut response_body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_transport_error)? {
            if response_body.len().saturating_add(chunk.len()) > MAX_FINALIZE_RESPONSE_BYTES {
                return Err(DesktopInstantFinalizeError::ResponseTooLarge);
            }
            response_body.extend_from_slice(&chunk);
        }
        let receipt = serde_json::from_slice::<InstantFinalizeReceiptV1>(&response_body)
            .map_err(|_| DesktopInstantFinalizeError::MalformedResponse)?;
        receipt
            .validate_for(&wire)
            .map_err(|_| DesktopInstantFinalizeError::InvalidResponse)?;

        match (status, receipt.state) {
            (202, InstantFinalizeStateV1::Pending) => Ok(InstantFinalizeDispatchOutcome::Pending),
            (200, InstantFinalizeStateV1::Published) => {
                let publication_id = receipt
                    .publication_id
                    .as_deref()
                    .ok_or(DesktopInstantFinalizeError::InvalidResponse)
                    .and_then(|value| {
                        InstantPublicationId::from_canonical_uuid(value)
                            .map_err(|_| DesktopInstantFinalizeError::InvalidResponse)
                    })?;
                let receipt = InstantFinalizeReceipt::new(request, publication_id)
                    .map_err(|_| DesktopInstantFinalizeError::InvalidResponse)?;
                Ok(InstantFinalizeDispatchOutcome::Published(Box::new(receipt)))
            }
            _ => Err(DesktopInstantFinalizeError::InvalidResponse),
        }
    }
}

pub fn instant_finalize_wire_request(
    tenant_id: &str,
    video_id: &str,
    operation_id: InstantOperationId,
    request: &InstantFinalizeRequest,
) -> Result<InstantFinalizeRequestV1, DesktopInstantFinalizeError> {
    let multipart = request.multipart();
    InstantFinalizeRequestV1::new(
        tenant_id.to_owned(),
        request.session_id().to_canonical_uuid(),
        operation_id.to_canonical_uuid(),
        multipart.upload_id.to_canonical_uuid(),
        video_id.to_owned(),
        multipart.ordered_parts_digest.to_hex(),
        multipart.object.object_version.to_hex(),
        request.job_id().to_canonical_uuid(),
        request.job_generation(),
    )
    .map_err(|_| DesktopInstantFinalizeError::InvalidContract)
}

fn valid_bearer_token(value: &str) -> bool {
    (32..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
}

fn validate_response_headers(headers: &HeaderMap) -> Result<(), DesktopInstantFinalizeError> {
    if headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_FINALIZE_RESPONSE_BYTES)
    {
        return Err(DesktopInstantFinalizeError::ResponseTooLarge);
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type != Some("application/json") {
        return Err(DesktopInstantFinalizeError::UnsupportedContentType);
    }
    Ok(())
}

fn map_transport_error(error: reqwest::Error) -> DesktopInstantFinalizeError {
    if error.is_timeout() {
        DesktopInstantFinalizeError::DeadlineExceeded
    } else {
        DesktopInstantFinalizeError::TransportUnavailable
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    use frame_media::{
        InstantCodec, InstantContainer, InstantFence, InstantJobId, InstantManifest,
        InstantMultipartBinding, InstantMultipartCompleteReceipt, InstantObjectId,
        InstantPartReceipt, InstantSegmentDescriptor, InstantSessionId, InstantTrackMetadata,
        InstantTrackRole, InstantUploadId, Sha256Digest, strong_sha256,
    };

    use super::*;
    use frame_authenticated_client::INSTANT_FINALIZE_SCHEMA_VERSION;

    const TENANT_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900001";
    const VIDEO_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900005";

    fn id(marker: u8) -> [u8; 16] {
        [marker; 16]
    }

    fn finalize_request() -> InstantFinalizeRequest {
        let session_id = InstantSessionId::from_csprng(id(2)).expect("session");
        let segment_bytes = b"bounded-instant-fragment";
        let segment = InstantSegmentDescriptor::new(
            session_id,
            0,
            0,
            1_000_000_000,
            true,
            InstantContainer::FragmentedMp4Cmaf,
            vec![
                InstantTrackMetadata::new(
                    1,
                    InstantTrackRole::ScreenVideo,
                    InstantCodec::H264Avc,
                    90_000,
                    30,
                    0,
                    1_000_000_000,
                )
                .expect("track"),
            ],
            segment_bytes.len() as u64,
            strong_sha256(segment_bytes),
        )
        .expect("segment");
        let manifest =
            InstantManifest::from_segments(session_id, [segment.clone()]).expect("manifest");
        let binding = InstantMultipartBinding {
            session_id,
            upload_id: InstantUploadId::from_csprng(id(4)).expect("upload"),
            expires_at_ns: 10_000_000_000,
            generation: 1,
            minimum_part_bytes: 1,
            maximum_part_bytes: 1024,
            maximum_parts: 8,
        }
        .validate()
        .expect("binding");
        let part =
            InstantPartReceipt::new(binding, &segment, strong_sha256(b"provider-part-receipt"))
                .expect("part");
        let multipart = InstantMultipartCompleteReceipt::new(
            binding,
            &manifest,
            &[part],
            InstantObjectId::from_csprng(id(6)).expect("object"),
            strong_sha256(b"object-version"),
        )
        .expect("multipart");
        InstantFinalizeRequest::new(
            session_id,
            7,
            InstantFence::new(1).expect("fence"),
            manifest,
            multipart,
            InstantJobId::from_csprng(id(7)).expect("job"),
            1,
        )
        .expect("request")
    }

    #[test]
    fn native_request_maps_to_shared_wire_contract_without_runtime_handles() {
        let request = finalize_request();
        let operation = InstantOperationId::from_csprng(id(3)).expect("operation");
        let wire = instant_finalize_wire_request(TENANT_ID, VIDEO_ID, operation, &request)
            .expect("wire request");
        assert_eq!(wire.schema_version, INSTANT_FINALIZE_SCHEMA_VERSION);
        assert_eq!(wire.session_id, request.session_id().to_canonical_uuid());
        assert_eq!(
            wire.upload_id,
            request.multipart().upload_id.to_canonical_uuid()
        );
        assert_eq!(wire.job_id, request.job_id().to_canonical_uuid());
        assert_eq!(
            wire.object_version,
            request.multipart().object.object_version.to_hex()
        );
        wire.validate().expect("valid wire request");
        let debug = format!("{wire:?}");
        assert!(!debug.contains(TENANT_ID));
        assert!(!debug.contains(&wire.request_sha256));
    }

    #[tokio::test]
    async fn native_http_caller_sends_bound_headers_and_accepts_published_receipt() {
        let request = finalize_request();
        let operation = InstantOperationId::from_csprng(id(3)).expect("operation");
        let wire = instant_finalize_wire_request(TENANT_ID, VIDEO_ID, operation, &request)
            .expect("wire request");
        let publication_id = "08080808-0808-0808-0808-080808080808";
        let receipt = InstantFinalizeReceiptV1 {
            schema_version: 1,
            state: InstantFinalizeStateV1::Published,
            request_sha256: wire.request_sha256.clone(),
            publication_id: Some(publication_id.into()),
            job_id: wire.job_id.clone(),
            job_generation: wire.job_generation,
            upload_id: wire.upload_id.clone(),
            object_version: wire.object_version.clone(),
            distribution_eligible: true,
        };
        let response_body = serde_json::to_vec(&receipt).expect("response JSON");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(false).expect("blocking listener");
        let address = listener.local_addr().expect("address");
        let expected_path = format!("/api/v1/instant-recordings/{}/finalize", wire.session_id);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            let mut request_bytes = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = stream.read(&mut buffer).expect("read request");
                assert_ne!(read, 0, "request ended before headers");
                request_bytes.extend_from_slice(&buffer[..read]);
                if request_bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request_bytes).to_ascii_lowercase();
            assert!(
                request_text
                    .starts_with(&format!("post {expected_path} http/1.1").to_ascii_lowercase())
            );
            assert!(request_text.contains(&format!("x-frame-tenant-id: {TENANT_ID}")));
            assert!(
                request_text.contains("authorization: bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            );
            assert!(request_text.contains(&format!(
                "idempotency-key: instant-finalize:{}",
                operation.to_canonical_uuid()
            )));
            let headers = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response_body.len()
            );
            stream.write_all(headers.as_bytes()).expect("headers");
            stream.write_all(&response_body).expect("body");
        });

        let origin =
            FrameOrigin::for_local_testing(&format!("http://{address}")).expect("local origin");
        let client = DesktopInstantFinalizeClient::new(origin).expect("client");
        let outcome = client
            .dispatch(
                TENANT_ID,
                VIDEO_ID,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                operation,
                &request,
            )
            .await
            .expect("published outcome");
        server.join().expect("server");
        let InstantFinalizeDispatchOutcome::Published(native_receipt) = outcome else {
            panic!("expected published receipt");
        };
        assert_eq!(native_receipt.job_id, request.job_id());
        assert_eq!(native_receipt.object, request.multipart().object);
    }

    #[test]
    fn canonical_uuid_rehydration_rejects_aliases_and_nil() {
        let digest = Sha256Digest::from_hex(&"a".repeat(64)).expect("digest");
        assert_eq!(digest.to_hex(), "a".repeat(64));
        assert!(
            InstantPublicationId::from_canonical_uuid("08080808-0808-0808-0808-080808080808")
                .is_ok()
        );
        assert!(
            InstantPublicationId::from_canonical_uuid("08080808-0808-0808-0808-08080808080A")
                .is_err()
        );
        assert!(
            InstantPublicationId::from_canonical_uuid("00000000-0000-0000-0000-000000000000")
                .is_err()
        );
    }
}
