//! Authenticated desktop adapter for the Instant finalize boundary.

use std::{fmt, time::Duration};

use frame_authenticated_client::{
    InstantFinalizeReceiptV1, InstantFinalizeRequestV1, InstantFinalizeStateV1,
};
use frame_client::FrameOrigin;
#[cfg(feature = "instant-finalize")]
use frame_client::{InstantUiErrorCodeV1, InstantUiPhaseV1, InstantUiProgressV1};
#[cfg(feature = "instant-finalize")]
use frame_media::{
    INSTANT_PROGRESS_VERSION, InstantFinalizeReceipt, InstantFinalizeRequest, InstantOperationId,
    InstantProgress, InstantPublicErrorCode, InstantPublicationId, InstantReadiness,
};
use reqwest::{
    Client,
    header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue, LOCATION},
    redirect::Policy,
};
use thiserror::Error;

const FINALIZE_DEADLINE: Duration = Duration::from_secs(15);
const MAX_FINALIZE_RESPONSE_BYTES: usize = 64 * 1024;

#[cfg(feature = "instant-finalize")]
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

/// Hardened native caller available to the desktop service layer. Redirects
/// and ambient proxies are disabled so authorization cannot be forwarded
/// outside the validated Frame origin.
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

    /// Sends one already validated, native-owned wire request. The bearer and
    /// request never originate in WebView IPC. The returned receipt is bound
    /// to the exact request before this method returns.
    pub async fn dispatch_wire(
        &self,
        bearer_token: &str,
        wire: &InstantFinalizeRequestV1,
    ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
        if !valid_bearer_token(bearer_token) {
            return Err(DesktopInstantFinalizeError::InvalidCredential);
        }
        wire.validate()
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let body =
            serde_json::to_vec(wire).map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let endpoint = self
            .origin
            .api_v1_url(&format!("instant-recordings/{}/finalize", wire.session_id))
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {bearer_token}"))
            .map_err(|_| DesktopInstantFinalizeError::InvalidCredential)?;
        authorization.set_sensitive(true);
        let mut tenant = HeaderValue::from_str(&wire.tenant_id)
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        let operation_key = format!("instant-finalize:{}", wire.operation_id);
        let mut idempotency = HeaderValue::from_str(&operation_key)
            .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?;
        tenant.set_sensitive(true);
        idempotency.set_sensitive(true);

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
            .validate_for(wire)
            .map_err(|_| DesktopInstantFinalizeError::InvalidResponse)?;

        match (status, receipt.state) {
            (202, InstantFinalizeStateV1::Pending) | (200, InstantFinalizeStateV1::Published) => {
                Ok(receipt)
            }
            _ => Err(DesktopInstantFinalizeError::InvalidResponse),
        }
    }

    /// Maps a sealed native request into the portable wire contract, sends it,
    /// and reconstructs the native receipt. This bridge stays optional so a
    /// normal Tauri build cannot acquire the media/GStreamer dependency tree.
    #[cfg(feature = "instant-finalize")]
    pub async fn dispatch(
        &self,
        tenant_id: &str,
        video_id: &str,
        bearer_token: &str,
        operation_id: InstantOperationId,
        request: &InstantFinalizeRequest,
    ) -> Result<InstantFinalizeDispatchOutcome, DesktopInstantFinalizeError> {
        let wire = instant_finalize_wire_request(tenant_id, video_id, operation_id, request)?;
        let receipt = self.dispatch_wire(bearer_token, &wire).await?;
        match receipt.state {
            InstantFinalizeStateV1::Pending => Ok(InstantFinalizeDispatchOutcome::Pending),
            InstantFinalizeStateV1::Published => {
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
        }
    }
}

#[cfg(feature = "instant-finalize")]
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

pub(crate) fn valid_bearer_token(value: &str) -> bool {
    (32..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
}

/// Projects the native journal status into the exact public-safe UI contract.
/// Native identifiers, hashes, byte totals, and credentials cannot cross this
/// adapter because the target type has no fields for them.
#[cfg(feature = "instant-finalize")]
pub fn project_instant_progress(
    progress: InstantProgress,
) -> Result<InstantUiProgressV1, DesktopInstantFinalizeError> {
    let expected_basis_points = if progress.segment_count == 0 {
        0
    } else {
        u16::try_from(
            u64::from(progress.verified_segment_count) * 10_000 / u64::from(progress.segment_count),
        )
        .map_err(|_| DesktopInstantFinalizeError::InvalidContract)?
    };
    if progress.version != INSTANT_PROGRESS_VERSION
        || progress.upload_basis_points > 10_000
        || progress.verified_segment_count > progress.segment_count
        || progress.upload_basis_points != expected_basis_points
        || (matches!(
            progress.readiness,
            InstantReadiness::Finalizing | InstantReadiness::ShareReady
        ) && (progress.segment_count == 0
            || progress.verified_segment_count != progress.segment_count
            || progress.upload_basis_points != 10_000))
    {
        return Err(DesktopInstantFinalizeError::InvalidContract);
    }
    let phase = match progress.readiness {
        InstantReadiness::Recording => InstantUiPhaseV1::Recording,
        InstantReadiness::LocallyRecoverable => InstantUiPhaseV1::LocallyRecoverable,
        InstantReadiness::Uploading => InstantUiPhaseV1::Uploading,
        InstantReadiness::Finalizing => InstantUiPhaseV1::Finalizing,
        InstantReadiness::ShareReady => InstantUiPhaseV1::ShareReady,
        InstantReadiness::Cancelled => InstantUiPhaseV1::Cancelled,
        InstantReadiness::RecoveryRequired => InstantUiPhaseV1::RecoveryRequired,
    };
    let error = progress.public_error.map(|error| match error {
        InstantPublicErrorCode::LocalStorageFull => InstantUiErrorCodeV1::LocalStorageFull,
        InstantPublicErrorCode::LocalStorageUnavailable => {
            InstantUiErrorCodeV1::LocalStorageUnavailable
        }
        InstantPublicErrorCode::NetworkOffline => InstantUiErrorCodeV1::NetworkOffline,
        InstantPublicErrorCode::UploadDelayed => InstantUiErrorCodeV1::UploadDelayed,
        InstantPublicErrorCode::UploadExpired => InstantUiErrorCodeV1::UploadExpired,
        InstantPublicErrorCode::FinalizeDelayed => InstantUiErrorCodeV1::FinalizeDelayed,
        InstantPublicErrorCode::RecordingRecoveryRequired => {
            InstantUiErrorCodeV1::RecordingRecoveryRequired
        }
        InstantPublicErrorCode::RecordingCancelled => InstantUiErrorCodeV1::RecordingCancelled,
        InstantPublicErrorCode::RecordingFailed => InstantUiErrorCodeV1::RecordingFailed,
    });
    let progress_basis_points = match phase {
        InstantUiPhaseV1::Uploading
        | InstantUiPhaseV1::Finalizing
        | InstantUiPhaseV1::ShareReady => Some(progress.upload_basis_points),
        InstantUiPhaseV1::Recording
        | InstantUiPhaseV1::LocallyRecoverable
        | InstantUiPhaseV1::Cancelled
        | InstantUiPhaseV1::RecoveryRequired => None,
    };
    InstantUiProgressV1::new(phase, progress_basis_points, progress.retrying, error)
        .map_err(|_| DesktopInstantFinalizeError::InvalidContract)
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

    #[cfg(feature = "instant-finalize")]
    use frame_media::{
        InstantCodec, InstantContainer, InstantFence, InstantJobId, InstantManifest,
        InstantMultipartBinding, InstantMultipartCompleteReceipt, InstantObjectId,
        InstantPartReceipt, InstantSegmentDescriptor, InstantSessionId, InstantTrackMetadata,
        InstantTrackRole, InstantUploadId, Sha256Digest, strong_sha256,
    };

    use super::*;
    #[cfg(feature = "instant-finalize")]
    use frame_authenticated_client::INSTANT_FINALIZE_SCHEMA_VERSION;

    const TENANT_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900001";
    const VIDEO_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900005";

    fn wire_request() -> InstantFinalizeRequestV1 {
        InstantFinalizeRequestV1::new(
            TENANT_ID.into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900002".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900003".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900004".into(),
            VIDEO_ID.into(),
            "a".repeat(64),
            "b".repeat(64),
            "018f47a6-7b1c-7f55-8f39-8f8a86900007".into(),
            1,
        )
        .expect("wire request")
    }

    #[cfg(feature = "instant-finalize")]
    fn id(marker: u8) -> [u8; 16] {
        [marker; 16]
    }

    #[cfg(feature = "instant-finalize")]
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

    #[cfg(feature = "instant-finalize")]
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
    async fn wire_http_caller_sends_bound_headers_and_accepts_published_receipt() {
        let wire = wire_request();
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
        let expected_operation_id = wire.operation_id.clone();
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
                expected_operation_id
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
        let returned = client
            .dispatch_wire("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &wire)
            .await
            .expect("published receipt");
        server.join().expect("server");
        assert_eq!(returned, receipt);
    }

    #[cfg(feature = "instant-finalize")]
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

    #[cfg(feature = "instant-finalize")]
    #[test]
    fn native_progress_projection_cannot_expose_native_identity_or_byte_counts() {
        let progress = InstantProgress {
            version: 1,
            readiness: InstantReadiness::Uploading,
            segment_count: 9,
            verified_segment_count: 4,
            retained_spool_bytes: 9_876_543,
            total_media_bytes: 12_345_678,
            upload_basis_points: 4_444,
            network_available: true,
            retrying: true,
            public_error: Some(InstantPublicErrorCode::UploadDelayed),
        };
        let projected = project_instant_progress(progress).expect("public projection");
        assert_eq!(projected.phase, InstantUiPhaseV1::Uploading);
        assert_eq!(projected.progress_basis_points, Some(4_444));
        let json = serde_json::to_string(&projected).expect("projection JSON");
        assert!(!json.contains("9876543"));
        assert!(!json.contains("12345678"));
        assert!(!json.contains("segment"));
    }

    #[cfg(feature = "instant-finalize")]
    #[test]
    fn native_progress_projection_rejects_future_versions_and_false_share_ready() {
        let mut progress = InstantProgress {
            version: INSTANT_PROGRESS_VERSION + 1,
            readiness: InstantReadiness::Uploading,
            segment_count: 2,
            verified_segment_count: 1,
            retained_spool_bytes: 10,
            total_media_bytes: 20,
            upload_basis_points: 5_000,
            network_available: true,
            retrying: false,
            public_error: None,
        };
        assert_eq!(
            project_instant_progress(progress),
            Err(DesktopInstantFinalizeError::InvalidContract)
        );

        progress.version = INSTANT_PROGRESS_VERSION;
        progress.upload_basis_points = 9_000;
        assert_eq!(
            project_instant_progress(progress),
            Err(DesktopInstantFinalizeError::InvalidContract)
        );

        progress.upload_basis_points = 5_000;
        progress.readiness = InstantReadiness::Finalizing;
        assert_eq!(
            project_instant_progress(progress),
            Err(DesktopInstantFinalizeError::InvalidContract)
        );

        progress.readiness = InstantReadiness::ShareReady;
        assert_eq!(
            project_instant_progress(progress),
            Err(DesktopInstantFinalizeError::InvalidContract)
        );

        progress.verified_segment_count = 2;
        progress.upload_basis_points = 10_000;
        let projected = project_instant_progress(progress).expect("truthful share ready");
        assert_eq!(projected.phase, InstantUiPhaseV1::ShareReady);
        assert_eq!(projected.progress_basis_points, Some(10_000));
    }
}
