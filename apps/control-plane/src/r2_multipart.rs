//! Restart-safe R2 multipart provider adapter.
//!
//! Provider upload IDs and part receipts are persisted in D1. Completion
//! streams the finished object through SHA-256 before committing its receipt;
//! a trusted probe is injected explicitly and cannot be supplied by clients.

use async_trait::async_trait;
use frame_domain::{
    AudioCodecV1, ByteSize, ChecksumSha256, ContentType, CorrelationId, DurationMillis,
    MediaContainerV1, MultipartLimitsV1, MultipartPartNumberV1, MultipartUploadId,
    MultipartUploadSpecV1, ObjectRevision, ScopedObjectKey, StorageFileExtension, TenantId,
    TimestampMillis, TrustedMediaProbeV1, UserId, VideoCodecV1, VideoId, VideoObjectDescriptor,
};
use frame_ports::{
    DownloadValidatorV1, MultipartObjectStoreV1, MultipartProviderCapabilitiesV1,
    ProviderAbortDispositionV1, ProviderAbortReceiptV1, ProviderCompleteMultipartRequestV1,
    ProviderCompletedObjectV1, ProviderCreateMultipartRequestV1, ProviderDownloadBodyV1,
    ProviderDownloadMetadataV1, ProviderDownloadRequestV1, ProviderDownloadResponseV1,
    ProviderEntityTag, ProviderLookupMultipartRequestV1, ProviderMultipartHandleV1,
    ProviderMultipartSessionV1, ProviderObjectVersion, ProviderPartReceiptV1, ProviderPartsListV1,
    ProviderPutPartRequestV1, ProviderUploadReferenceV1, StorageFailure, StorageFailureKind,
    StorageRequestContext,
};
use futures::TryStreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{
    Bucket, D1Database, HttpMetadata, UploadedPart,
    send::{IntoSendFuture, SendWrapper},
};

const MIN_PART_BYTES: u64 = 5 * 1_024 * 1_024;
const MAX_PART_BYTES: u64 = 100 * 1_024 * 1_024;
const MAX_TOTAL_BYTES: u64 = 5_000_000_000_000;
const MAX_RANGE_BYTES: u64 = 16 * 1_024 * 1_024;
const ABORT_RETRY_BASE_MS: i64 = 1_000;
const ABORT_RETRY_MAX_MS: i64 = 300_000;
const ABORT_ATTEMPT_LOCK_MS: i64 = 60_000;
const COMPLETION_RETRY_BASE_MS: i64 = 60_000;
const COMPLETION_RETRY_MAX_MS: i64 = 60 * 60 * 1_000;
const MAX_COMPLETION_RECONCILIATION_ATTEMPTS: i64 = 12;
const MAX_SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const PART_CLAIM_LEASE_MS: i64 = 5 * 60 * 1_000;
const COMPLETION_CLAIM_LEASE_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedAbortOutcomeV1 {
    Confirmed { attempt: i64 },
    PreservedObject { attempt: i64 },
    AlreadyAborted,
    AlreadyCompleted,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortIntentKind {
    ExpiryCleanup,
    AuthenticatedDelete,
}

impl AbortIntentKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExpiryCleanup => "expiry_cleanup",
            Self::AuthenticatedDelete => "authenticated_delete",
        }
    }
}

#[derive(Debug)]
enum AbortProviderAttempt {
    Terminal(AbortTerminalOutcome),
    RetryableFailure(StorageFailure),
}

#[async_trait]
pub trait TrustedR2MediaProbeV1: Send + Sync {
    async fn probe(
        &self,
        bucket: &Bucket,
        key: &ScopedObjectKey,
        content_type: &ContentType,
        size: ByteSize,
        checksum: &ChecksumSha256,
    ) -> Result<TrustedMediaProbeV1, StorageFailure>;
}

/// Production trusted-probe boundary. Multipart completion can finish R2
/// before the native probe job has committed its verified D1 result; that
/// state is deliberately retryable and never falls back to client metadata.
#[derive(Debug)]
pub struct D1TrustedMediaProbeV1<'a> {
    database: &'a D1Database,
}

impl<'a> D1TrustedMediaProbeV1<'a> {
    #[must_use]
    pub const fn new(database: &'a D1Database) -> Self {
        Self { database }
    }
}

#[async_trait]
impl TrustedR2MediaProbeV1 for D1TrustedMediaProbeV1<'_> {
    async fn probe(
        &self,
        _bucket: &Bucket,
        key: &ScopedObjectKey,
        content_type: &ContentType,
        size: ByteSize,
        checksum: &ChecksumSha256,
    ) -> Result<TrustedMediaProbeV1, StorageFailure> {
        let row = self
            .database
            .prepare(
                "SELECT p.container,p.video_codec,p.audio_codec,p.width,p.height,p.duration_ms,\
                 p.frame_rate_numerator,p.frame_rate_denominator \
                 FROM media_source_probes_v1 p JOIN video_uploads u \
                   ON u.organization_id=p.organization_id AND u.video_id=p.video_id \
                  AND u.source_version=p.source_version AND u.source_object_key=p.source_object_key \
                 WHERE p.organization_id=?1 AND p.source_object_key=?2 \
                   AND p.source_checksum_sha256=?3 AND p.source_bytes=?4 \
                   AND p.source_content_type=?5 AND p.trust='verified_native_probe' \
                   AND p.state='verified' LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&key.tenant_id().to_string()),
                JsValue::from_str(key.as_str()),
                JsValue::from_str(checksum.as_str()),
                number(size.get())?,
                JsValue::from_str(content_type.as_str()),
            ])
            .map_err(map_worker_error)?
            .first::<TrustedProbeRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(unavailable)?;
        let numerator = u64::try_from(row.frame_rate_numerator).map_err(|_| integrity())?;
        let denominator = u64::try_from(row.frame_rate_denominator).map_err(|_| integrity())?;
        let rate = numerator
            .checked_mul(1_000)
            .and_then(|value| value.checked_div(denominator))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(integrity)?;
        TrustedMediaProbeV1::new(
            parse_container(&row.container)?,
            parse_video_codec(&row.video_codec)?,
            parse_audio_codec(&row.audio_codec)?,
            u16::try_from(row.width).map_err(|_| integrity())?,
            u16::try_from(row.height).map_err(|_| integrity())?,
            u64::try_from(row.duration_ms).map_err(|_| integrity())?,
            rate,
        )
        .map_err(|_| integrity())
    }
}

#[derive(Debug)]
struct LocalTrustedMediaProbeV1;

#[async_trait]
impl TrustedR2MediaProbeV1 for LocalTrustedMediaProbeV1 {
    async fn probe(
        &self,
        _bucket: &Bucket,
        _key: &ScopedObjectKey,
        _content_type: &ContentType,
        _size: ByteSize,
        _checksum: &ChecksumSha256,
    ) -> Result<TrustedMediaProbeV1, StorageFailure> {
        TrustedMediaProbeV1::new(
            MediaContainerV1::Webm,
            VideoCodecV1::Vp9,
            AudioCodecV1::Opus,
            1_280,
            720,
            1_000,
            30_000,
        )
        .map_err(|_| integrity())
    }
}

/// Exercises the production R2/D1 multipart adapter against Wrangler's local
/// bindings. The caller is an exact loopback-only route; no provider
/// credentials or externally reachable authority enters this contract.
pub async fn run_local_contract(
    bucket: &Bucket,
    database: &D1Database,
) -> Result<(), StorageFailure> {
    const PART_SIZE: u64 = 5 * 1_024 * 1_024;
    const TAIL_SIZE: usize = 4_096;
    let started_at = now()?;
    let expires_at = started_at
        // Pure-Wasm hashing of the provider's minimum 5 MiB part can be slow
        // on constrained CI hosts. The contract still stays well below the
        // production adapter's 24-hour session ceiling.
        .checked_add(DurationMillis::new(600_000).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let tenant_id = TenantId::new();
    let user_id = UserId::new();
    let video_id = VideoId::new();
    let integration_id = CorrelationId::new();
    let upload_ids = [
        MultipartUploadId::new(),
        MultipartUploadId::new(),
        MultipartUploadId::new(),
        MultipartUploadId::new(),
    ];
    let keys = [1_u64, 2, 3, 4]
        .into_iter()
        .map(|revision| {
            ScopedObjectKey::source(
                tenant_id,
                video_id,
                ObjectRevision::new(revision).map_err(|_| invalid())?,
                VideoObjectDescriptor::Source {
                    extension: StorageFileExtension::parse("webm").map_err(|_| invalid())?,
                },
            )
            .map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut first_part = vec![0_u8; usize::try_from(PART_SIZE).map_err(|_| invalid())?];
    for (index, byte) in first_part.iter_mut().enumerate() {
        *byte = u8::try_from(index % 251).map_err(|_| invalid())?;
    }
    let tail = vec![0xa5; TAIL_SIZE];
    let mut complete_bytes = first_part.clone();
    complete_bytes.extend_from_slice(&tail);
    let total_size = ByteSize::new(u64::try_from(complete_bytes.len()).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let full_checksum = ChecksumSha256::digest_bytes(&complete_bytes);
    let content_type = ContentType::parse("video/webm").map_err(|_| invalid())?;
    seed_local_contract(
        database,
        LocalSeedV1 {
            tenant_id,
            user_id,
            video_id,
            integration_id,
            upload_ids,
            keys: [&keys[0], &keys[1], &keys[2], &keys[3]],
            expected_bytes: total_size,
            checksum: &full_checksum,
            content_type: &content_type,
            part_size: PART_SIZE,
            part_count: 2,
            created_at: started_at,
            expires_at,
        },
    )
    .await?;
    let limits = MultipartLimitsV1::new(
        ByteSize::new(PART_SIZE).map_err(|_| invalid())?,
        ByteSize::new(MAX_PART_BYTES).map_err(|_| invalid())?,
        10_000,
        ByteSize::new(MAX_TOTAL_BYTES).map_err(|_| invalid())?,
        ByteSize::new(MAX_PART_BYTES).map_err(|_| invalid())?,
        DurationMillis::new(u64::try_from(MAX_SESSION_TTL_MS).map_err(|_| invalid())?)
            .map_err(|_| invalid())?,
    )
    .map_err(|_| invalid())?;
    let specs = keys
        .iter()
        .map(|key| {
            MultipartUploadSpecV1::new(
                key.clone(),
                total_size,
                ByteSize::new(PART_SIZE).map_err(|_| invalid())?,
                full_checksum.clone(),
                content_type.clone(),
                limits,
            )
            .map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let probe = LocalTrustedMediaProbeV1;
    let store = R2MultipartObjectStoreV1::new(bucket, database, &probe)?;

    let context = StorageRequestContext::new(tenant_id, correlation_for(upload_ids[0])?);
    let create = ProviderCreateMultipartRequestV1::new(
        upload_ids[0],
        specs[0].clone(),
        expires_at,
        context.correlation_id(),
    );
    let (first_create, concurrent_create) = futures::join!(
        store.create_multipart(context, create.clone()),
        store.create_multipart(context, create.clone()),
    );
    let session = match (first_create, concurrent_create) {
        (Ok(first), Ok(second)) if first == second => first,
        (Ok(session), Err(error)) | (Err(error), Ok(session))
            if error.kind() == StorageFailureKind::Unavailable =>
        {
            session
        }
        _ => return Err(integrity()),
    };
    if store.create_multipart(context, create).await? != session
        || database
            .prepare(
                "SELECT 1 AS present FROM r2_multipart_creation_claims_v1 \
                 WHERE upload_id=?1 AND state='committed' \
                   AND provider_upload_id IS NOT NULL LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_ids[0].to_string())])
            .map_err(map_worker_error)?
            .first::<PresenceRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_none_or(|row| row.present != 1)
    {
        return Err(integrity());
    }
    let reference = ProviderUploadReferenceV1::new(
        upload_ids[0],
        keys[0].clone(),
        session.handle().clone(),
        context.correlation_id(),
    );
    let first_request = ProviderPutPartRequestV1::new(
        reference.clone(),
        MultipartPartNumberV1::new(1).map_err(|_| invalid())?,
        ChecksumSha256::digest_bytes(&first_part),
        first_part.clone(),
    );
    let first_receipt = store.put_part(context, first_request.clone()).await?;
    if store.put_part(context, first_request).await? != first_receipt {
        return Err(integrity());
    }
    let second_receipt = store
        .put_part(
            context,
            ProviderPutPartRequestV1::new(
                reference.clone(),
                MultipartPartNumberV1::new(2).map_err(|_| invalid())?,
                ChecksumSha256::digest_bytes(&tail),
                tail.clone(),
            ),
        )
        .await?;
    let listed = store.list_parts(context, reference.clone()).await?;
    if listed.parts() != [first_receipt.clone(), second_receipt.clone()] {
        return Err(integrity());
    }
    let completion_request = ProviderCompleteMultipartRequestV1::new(
        reference.clone(),
        listed.parts().to_vec(),
        total_size,
        full_checksum.clone(),
        content_type.clone(),
    )?;
    let (first_completion, concurrent_completion) = futures::join!(
        store.complete_multipart(context, completion_request.clone()),
        store.complete_multipart(context, completion_request.clone()),
    );
    let completed = match (first_completion, concurrent_completion) {
        (Ok(first), Ok(second)) if first == second => first,
        (Ok(completed), Err(error)) | (Err(error), Ok(completed))
            if error.kind() == StorageFailureKind::Unavailable =>
        {
            completed
        }
        _ => return Err(integrity()),
    };
    if store
        .complete_multipart(context, completion_request)
        .await?
        != completed
        || completed.size() != total_size
        || completed.checksum_sha256() != &full_checksum
        || database
            .prepare(
                "SELECT 1 AS present FROM r2_multipart_completion_claims_v1 \
                 WHERE upload_id=?1 AND state='complete' AND attempt_count=1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_ids[0].to_string())])
            .map_err(map_worker_error)?
            .first::<PresenceRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_none_or(|row| row.present != 1)
    {
        return Err(integrity());
    }
    let head = store
        .head_private(
            context,
            ProviderDownloadRequestV1::new(
                keys[0].clone(),
                None,
                DownloadValidatorV1::None,
                context.correlation_id(),
            ),
        )
        .await?;
    if !matches!(head, ProviderDownloadResponseV1::Head(ref metadata)
        if metadata.size() == total_size && metadata.checksum_sha256() == &full_checksum)
    {
        return Err(integrity());
    }
    let range = frame_ports::ObjectByteRange::new(PART_SIZE - 32, PART_SIZE + 32)?;
    let ranged = store
        .get_private(
            context,
            ProviderDownloadRequestV1::new(
                keys[0].clone(),
                Some(range),
                DownloadValidatorV1::None,
                context.correlation_id(),
            ),
        )
        .await?;
    let ProviderDownloadResponseV1::Body {
        range: returned_range,
        mut body,
        ..
    } = ranged
    else {
        return Err(integrity());
    };
    let mut ranged_bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        ranged_bytes.extend_from_slice(&chunk);
    }
    if returned_range != range
        || ranged_bytes
            != complete_bytes[usize::try_from(range.start()).map_err(|_| invalid())?
                ..usize::try_from(range.end_exclusive()).map_err(|_| invalid())?]
    {
        return Err(integrity());
    }
    if store
        .abort_multipart(context, reference)
        .await?
        .disposition()
        != ProviderAbortDispositionV1::AlreadyCompleted
    {
        return Err(integrity());
    }

    let abort_context = StorageRequestContext::new(tenant_id, correlation_for(upload_ids[1])?);
    let abort_session = store
        .create_multipart(
            abort_context,
            ProviderCreateMultipartRequestV1::new(
                upload_ids[1],
                specs[1].clone(),
                expires_at,
                abort_context.correlation_id(),
            ),
        )
        .await?;
    let abort_reference = ProviderUploadReferenceV1::new(
        upload_ids[1],
        keys[1].clone(),
        abort_session.handle().clone(),
        abort_context.correlation_id(),
    );
    let mut conflicting_first_part = first_part.clone();
    conflicting_first_part[0] ^= 0xff;
    let (first_claim, conflicting_claim) = futures::join!(
        store.put_part(
            abort_context,
            ProviderPutPartRequestV1::new(
                abort_reference.clone(),
                MultipartPartNumberV1::new(1).map_err(|_| invalid())?,
                ChecksumSha256::digest_bytes(&first_part),
                first_part.clone(),
            ),
        ),
        store.put_part(
            abort_context,
            ProviderPutPartRequestV1::new(
                abort_reference.clone(),
                MultipartPartNumberV1::new(1).map_err(|_| invalid())?,
                ChecksumSha256::digest_bytes(&conflicting_first_part),
                conflicting_first_part,
            ),
        ),
    );
    let exactly_one_claim_won = matches!(
        (&first_claim, &conflicting_claim),
        (Ok(_), Err(error)) | (Err(error), Ok(_))
            if matches!(
                error.kind(),
                StorageFailureKind::PreconditionFailed | StorageFailureKind::Unavailable
            )
    );
    if !exactly_one_claim_won
        || store
            .list_parts(abort_context, abort_reference.clone())
            .await?
            .parts()
            .len()
            != 1
    {
        return Err(integrity());
    }
    if store
        .abort_multipart(abort_context, abort_reference.clone())
        .await?
        .disposition()
        != ProviderAbortDispositionV1::Aborted
        || store
            .abort_multipart(abort_context, abort_reference)
            .await?
            .disposition()
            != ProviderAbortDispositionV1::AlreadyAborted
    {
        return Err(integrity());
    }

    let race_context = StorageRequestContext::new(tenant_id, correlation_for(upload_ids[3])?);
    let race_session = store
        .create_multipart(
            race_context,
            ProviderCreateMultipartRequestV1::new(
                upload_ids[3],
                specs[3].clone(),
                expires_at,
                race_context.correlation_id(),
            ),
        )
        .await?;
    let race_reference = ProviderUploadReferenceV1::new(
        upload_ids[3],
        keys[3].clone(),
        race_session.handle().clone(),
        race_context.correlation_id(),
    );
    store
        .put_part(
            race_context,
            ProviderPutPartRequestV1::new(
                race_reference.clone(),
                MultipartPartNumberV1::new(1).map_err(|_| invalid())?,
                ChecksumSha256::digest_bytes(&first_part),
                first_part,
            ),
        )
        .await?;
    store
        .put_part(
            race_context,
            ProviderPutPartRequestV1::new(
                race_reference.clone(),
                MultipartPartNumberV1::new(2).map_err(|_| invalid())?,
                ChecksumSha256::digest_bytes(&tail),
                tail,
            ),
        )
        .await?;
    let race_parts = store
        .list_parts(race_context, race_reference.clone())
        .await?;
    let race_completion_request = ProviderCompleteMultipartRequestV1::new(
        race_reference.clone(),
        race_parts.parts().to_vec(),
        total_size,
        full_checksum.clone(),
        content_type.clone(),
    )?;
    let (race_completion, race_abort) = futures::join!(
        store.complete_multipart(race_context, race_completion_request),
        store.abort_multipart(race_context, race_reference.clone()),
    );
    match (race_completion, race_abort) {
        (Ok(object), Ok(abort))
            if object.checksum_sha256() == &full_checksum
                && abort.disposition() == ProviderAbortDispositionV1::AlreadyCompleted =>
        {
            bucket
                .delete(keys[3].as_str())
                .into_send()
                .await
                .map_err(map_worker_error)?;
        }
        (Ok(object), Err(error))
            if object.checksum_sha256() == &full_checksum
                && error.kind() == StorageFailureKind::Unavailable =>
        {
            if store
                .abort_multipart(race_context, race_reference.clone())
                .await?
                .disposition()
                != ProviderAbortDispositionV1::AlreadyCompleted
            {
                return Err(integrity());
            }
            bucket
                .delete(keys[3].as_str())
                .into_send()
                .await
                .map_err(map_worker_error)?;
        }
        (Err(error), Ok(abort))
            if matches!(
                error.kind(),
                StorageFailureKind::PreconditionFailed | StorageFailureKind::Unavailable
            ) && abort.disposition() == ProviderAbortDispositionV1::Aborted =>
        {
            if bucket
                .head(keys[3].as_str())
                .into_send()
                .await
                .map_err(map_worker_error)?
                .is_some()
                || store
                    .abort_multipart(race_context, race_reference.clone())
                    .await?
                    .disposition()
                    != ProviderAbortDispositionV1::AlreadyAborted
            {
                return Err(integrity());
            }
        }
        _ => return Err(integrity()),
    }

    let stale_context = StorageRequestContext::new(tenant_id, correlation_for(upload_ids[2])?);
    store
        .create_multipart(
            stale_context,
            ProviderCreateMultipartRequestV1::new(
                upload_ids[2],
                specs[2].clone(),
                expires_at,
                stale_context.correlation_id(),
            ),
        )
        .await?;
    let forced_expiry = now()?;
    let expiry_update = database
        .prepare(
            "UPDATE r2_multipart_sessions_v1 SET expires_at_ms=?2 \
             WHERE upload_id=?1 AND state='open' AND created_at_ms<?2",
        )
        .bind(&[
            JsValue::from_str(&upload_ids[2].to_string()),
            JsValue::from_f64(forced_expiry.get() as f64),
        ])
        .map_err(map_worker_error)?
        .run()
        .into_send()
        .await
        .map_err(map_worker_error)?;
    if expiry_update
        .meta()
        .map_err(map_worker_error)?
        .and_then(|meta| meta.changes)
        != Some(1)
        || store
            .claim_completion(upload_ids[2], &"ab".repeat(32), forced_expiry)
            .await
            .is_ok()
        || database
            .prepare(
                "SELECT 1 AS present FROM r2_multipart_completion_claims_v1 \
                 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_ids[2].to_string())])
            .map_err(map_worker_error)?
            .first::<PresenceRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_some()
        || store.cleanup_stale(forced_expiry, 100).await? == 0
        || store
            .lookup_multipart(
                stale_context,
                ProviderLookupMultipartRequestV1::new(
                    upload_ids[2],
                    keys[2].clone(),
                    stale_context.correlation_id(),
                ),
            )
            .await?
            .is_some()
    {
        return Err(integrity());
    }
    let wrong_context = StorageRequestContext::new(TenantId::new(), CorrelationId::new());
    if !matches!(
        store
            .create_multipart(
                wrong_context,
                ProviderCreateMultipartRequestV1::new(
                    upload_ids[0],
                    specs[0].clone(),
                    expires_at,
                    wrong_context.correlation_id(),
                ),
            )
            .await,
        Err(error) if error.kind() == StorageFailureKind::Unauthorized
    ) {
        return Err(integrity());
    }
    bucket
        .delete(keys[0].as_str())
        .into_send()
        .await
        .map_err(map_worker_error)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct LocalSeedV1<'a> {
    tenant_id: TenantId,
    user_id: UserId,
    video_id: VideoId,
    integration_id: CorrelationId,
    upload_ids: [MultipartUploadId; 4],
    keys: [&'a ScopedObjectKey; 4],
    expected_bytes: ByteSize,
    checksum: &'a ChecksumSha256,
    content_type: &'a ContentType,
    part_size: u64,
    part_count: u16,
    created_at: TimestampMillis,
    expires_at: TimestampMillis,
}

async fn seed_local_contract(
    database: &D1Database,
    seed: LocalSeedV1<'_>,
) -> Result<(), StorageFailure> {
    let capabilities = r#"{"conditional_put":true,"multipart":true,"schema_version":1}"#;
    let mut statements = vec![
        database
            .prepare(
                "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,'R2 conformance',?3,?3)",
            )
            .bind(&[
                JsValue::from_str(&seed.user_id.to_string()),
                JsValue::from_str(&format!("r2-{}@example.invalid", seed.user_id)),
                JsValue::from_f64(seed.created_at.get() as f64),
            ])
            .map_err(map_worker_error)?,
        database
            .prepare(
                "INSERT INTO organizations(id,owner_id,name,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,'R2 conformance',?3,?3)",
            )
            .bind(&[
                JsValue::from_str(&seed.tenant_id.to_string()),
                JsValue::from_str(&seed.user_id.to_string()),
                JsValue::from_f64(seed.created_at.get() as f64),
            ])
            .map_err(map_worker_error)?,
        database
            .prepare(
                "INSERT INTO organization_members(organization_id,user_id,role,state,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,'owner','active',?3,?3)",
            )
            .bind(&[
                JsValue::from_str(&seed.tenant_id.to_string()),
                JsValue::from_str(&seed.user_id.to_string()),
                JsValue::from_f64(seed.created_at.get() as f64),
            ])
            .map_err(map_worker_error)?,
        database
            .prepare(
                "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id) \
                 VALUES(?1,?2,'R2 conformance','uploading',?3,?3,?4)",
            )
            .bind(&[
                JsValue::from_str(&seed.video_id.to_string()),
                JsValue::from_str(&seed.user_id.to_string()),
                JsValue::from_f64(seed.created_at.get() as f64),
                JsValue::from_str(&seed.tenant_id.to_string()),
            ])
            .map_err(map_worker_error)?,
        database
            .prepare(
                "INSERT INTO storage_integrations(\
                 id,organization_id,owner_user_id,provider,state,capabilities_json,credential_ciphertext,\
                 created_at_ms,updated_at_ms,capabilities_checksum) \
                 VALUES(?1,?2,?3,'r2','active',?4,'local-conformance-sealed',?5,?5,?6)",
            )
            .bind(&[
                JsValue::from_str(&seed.integration_id.to_string()),
                JsValue::from_str(&seed.tenant_id.to_string()),
                JsValue::from_str(&seed.user_id.to_string()),
                JsValue::from_str(capabilities),
                JsValue::from_f64(seed.created_at.get() as f64),
                JsValue::from_str(&"01".repeat(32)),
            ])
            .map_err(map_worker_error)?,
    ];
    for ((upload_id, key), revision) in seed.upload_ids.into_iter().zip(seed.keys).zip(1_u16..=4) {
        statements.push(
            database
                .prepare(
                    "INSERT INTO video_uploads(\
                     id,organization_id,video_id,state,expected_bytes,received_bytes,idempotency_key,\
                     source_object_key,source_version,content_type,checksum_sha256,created_at_ms,updated_at_ms,\
                     revision,event_sequence,event_fingerprint,transfer_mode) \
                     VALUES(?1,?2,?3,'initiated',?4,0,?5,?6,?7,?8,NULL,?9,?9,0,0,?10,'brokered')",
                )
                .bind(&[
                    JsValue::from_str(&upload_id.to_string()),
                    JsValue::from_str(&seed.tenant_id.to_string()),
                    JsValue::from_str(&seed.video_id.to_string()),
                    number(seed.expected_bytes.get())?,
                    JsValue::from_str(&format!("r2-local-{upload_id}")),
                    JsValue::from_str(key.as_str()),
                    JsValue::from_f64(f64::from(revision)),
                    JsValue::from_str(seed.content_type.as_str()),
                    JsValue::from_f64(seed.created_at.get() as f64),
                    JsValue::from_str(
                        "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9",
                    ),
                ])
                .map_err(map_worker_error)?,
        );
        statements.push(
            database
                .prepare(
                    "INSERT INTO r2_multipart_intents_v1(\
                     upload_id,integration_id,checksum_sha256,part_size,part_count,expires_at_ms,created_at_ms) \
                     VALUES(?1,?2,?3,?4,?5,?6,?7)",
                )
                .bind(&[
                    JsValue::from_str(&upload_id.to_string()),
                    JsValue::from_str(&seed.integration_id.to_string()),
                    JsValue::from_str(seed.checksum.as_str()),
                    number(seed.part_size)?,
                    JsValue::from_f64(f64::from(seed.part_count)),
                    JsValue::from_f64(seed.expires_at.get() as f64),
                    JsValue::from_f64(seed.created_at.get() as f64),
                ])
                .map_err(map_worker_error)?,
        );
    }
    let results = database
        .batch(statements)
        .into_send()
        .await
        .map_err(map_worker_error)?;
    if results.len() == 13 && results.iter().all(worker::D1Result::success) {
        Ok(())
    } else {
        Err(unavailable())
    }
}

fn correlation_for(upload_id: MultipartUploadId) -> Result<CorrelationId, StorageFailure> {
    CorrelationId::parse(&upload_id.to_string()).map_err(|_| integrity())
}

#[derive(Debug)]
pub struct R2MultipartObjectStoreV1<'a, P: ?Sized> {
    bucket: &'a Bucket,
    database: &'a D1Database,
    probe: &'a P,
    capabilities: MultipartProviderCapabilitiesV1,
}

impl<'a, P: TrustedR2MediaProbeV1 + ?Sized> R2MultipartObjectStoreV1<'a, P> {
    pub fn new(
        bucket: &'a Bucket,
        database: &'a D1Database,
        probe: &'a P,
    ) -> Result<Self, StorageFailure> {
        Ok(Self {
            bucket,
            database,
            probe,
            capabilities: MultipartProviderCapabilitiesV1::full(
                ByteSize::new(MIN_PART_BYTES).map_err(|_| invalid())?,
                ByteSize::new(MAX_PART_BYTES).map_err(|_| invalid())?,
                10_000,
                ByteSize::new(MAX_TOTAL_BYTES).map_err(|_| invalid())?,
                ByteSize::new(MAX_RANGE_BYTES).map_err(|_| invalid())?,
                true,
            )?,
        })
    }

    fn authorize(
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<(), StorageFailure> {
        (context.tenant_id() == key.tenant_id())
            .then_some(())
            .ok_or_else(|| StorageFailure::new(StorageFailureKind::Unauthorized))
    }

    async fn provider_mutations_enabled(&self) -> Result<bool, StorageFailure> {
        self.database
            .prepare(
                "SELECT COUNT(*) AS present FROM r2_multipart_claim_rollout_v1 \
                 WHERE singleton=1 AND phase='enabled'",
            )
            .bind(&[])
            .map_err(map_worker_error)?
            .first::<PresenceRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)
            .map(|row| row.is_some_and(|row| row.present == 1))
    }

    async fn require_provider_mutations_enabled(&self) -> Result<(), StorageFailure> {
        if self.provider_mutations_enabled().await? {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    async fn session(
        &self,
        upload_id: MultipartUploadId,
    ) -> Result<Option<SessionRow>, StorageFailure> {
        self.database
            .prepare(
                "SELECT upload_id,object_key,provider_upload_id,state,expected_bytes,checksum_sha256,\
                 content_type,correlation_id,created_at_ms,expires_at_ms,completed_at_ms \
                 FROM r2_multipart_sessions_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_id.to_string())])
            .map_err(map_worker_error)?
            .first::<SessionRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)
    }

    async fn intent_geometry(
        &self,
        upload_id: MultipartUploadId,
    ) -> Result<MultipartIntentGeometryRow, StorageFailure> {
        self.database
            .prepare(
                "SELECT part_size,part_count FROM r2_multipart_intents_v1 \
                 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_id.to_string())])
            .map_err(map_worker_error)?
            .first::<MultipartIntentGeometryRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(integrity)
    }

    async fn creation_claim(
        &self,
        upload_id: MultipartUploadId,
    ) -> Result<Option<CreationClaimRow>, StorageFailure> {
        self.database
            .prepare(
                "SELECT upload_id,organization_id,object_key,expected_bytes,checksum_sha256,content_type,\
                 correlation_id,part_size,part_count,expires_at_ms,claim_token,state,\
                 provider_upload_id,created_at_ms \
                 FROM r2_multipart_creation_claims_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_id.to_string())])
            .map_err(map_worker_error)?
            .first::<CreationClaimRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)
    }

    fn exact_creation_claim(
        claim: &CreationClaimRow,
        context: StorageRequestContext,
        request: &ProviderCreateMultipartRequestV1,
    ) -> Result<bool, StorageFailure> {
        Ok(claim.upload_id == request.upload_id().to_string()
            && claim.organization_id == context.tenant_id().to_string()
            && claim.object_key == request.key().as_str()
            && claim.expected_bytes == signed(request.spec().total_size().get())?
            && claim.checksum_sha256 == request.spec().checksum_sha256().as_str()
            && claim.content_type == request.spec().content_type().as_str()
            && claim.correlation_id == context.correlation_id().to_string()
            && claim.part_size == signed(request.spec().part_size().get())?
            && claim.part_count == i64::from(request.spec().part_count())
            && claim.expires_at_ms == request.expires_at().get())
    }

    async fn reserve_provider_creation(
        &self,
        context: StorageRequestContext,
        request: &ProviderCreateMultipartRequestV1,
        current: TimestampMillis,
    ) -> Result<CreationReservation, StorageFailure> {
        let claim_token = CorrelationId::new().to_string();
        let result = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_creation_claims_v1(\
                 upload_id,organization_id,object_key,expected_bytes,checksum_sha256,content_type,\
                 correlation_id,part_size,part_count,expires_at_ms,claim_token,state,\
                 provider_upload_id,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'reserved',NULL,?12,?12) \
                 ON CONFLICT(upload_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.upload_id().to_string()),
                JsValue::from_str(&context.tenant_id().to_string()),
                JsValue::from_str(request.key().as_str()),
                number(request.spec().total_size().get())?,
                JsValue::from_str(request.spec().checksum_sha256().as_str()),
                JsValue::from_str(request.spec().content_type().as_str()),
                JsValue::from_str(&context.correlation_id().to_string()),
                number(request.spec().part_size().get())?,
                JsValue::from_f64(f64::from(request.spec().part_count())),
                JsValue::from_f64(request.expires_at().get() as f64),
                JsValue::from_str(&claim_token),
                JsValue::from_f64(current.get() as f64),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !result.success() {
            return Err(unavailable());
        }
        let acquired = result
            .meta()
            .map_err(map_worker_error)?
            .and_then(|meta| meta.changes)
            == Some(1);
        let claim = self
            .creation_claim(request.upload_id())
            .await?
            .ok_or_else(unavailable)?;
        if !Self::exact_creation_claim(&claim, context, request)? {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        if acquired && claim.claim_token != claim_token {
            return Err(integrity());
        }
        Ok(CreationReservation { claim, acquired })
    }

    async fn bind_provider_creation(
        &self,
        upload_id: MultipartUploadId,
        claim_token: &str,
        provider_upload_id: &str,
        current: TimestampMillis,
    ) -> Result<CreationClaimRow, StorageFailure> {
        let update = self
            .database
            .prepare(
                "UPDATE r2_multipart_creation_claims_v1 \
                 SET state='provider_bound',provider_upload_id=?3,updated_at_ms=?4 \
                 WHERE upload_id=?1 AND claim_token=?2 AND state='reserved'",
            )
            .bind(&[
                JsValue::from_str(&upload_id.to_string()),
                JsValue::from_str(claim_token),
                JsValue::from_str(provider_upload_id),
                JsValue::from_f64(current.get() as f64),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await;
        if let Ok(result) = &update
            && !result.success()
        {
            return Err(unavailable());
        }
        let claim = self
            .creation_claim(upload_id)
            .await?
            .ok_or_else(unavailable)?;
        if claim.claim_token == claim_token
            && matches!(claim.state.as_str(), "provider_bound" | "committed")
            && claim.provider_upload_id.as_deref() == Some(provider_upload_id)
        {
            Ok(claim)
        } else {
            update.map_err(map_worker_error)?;
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        }
    }

    async fn commit_provider_creation_claim(
        &self,
        claim: &CreationClaimRow,
        current: TimestampMillis,
    ) -> Result<SessionRow, StorageFailure> {
        let upload_id = MultipartUploadId::parse(&claim.upload_id).map_err(|_| integrity())?;
        let provider_upload_id = claim
            .provider_upload_id
            .as_deref()
            .ok_or_else(unavailable)?;
        if claim.state == "provider_bound" {
            let results = self
                .database
                .batch(vec![
                    self.database
                        .prepare(
                            "INSERT INTO r2_multipart_sessions_v1(\
                             upload_id,object_key,provider_upload_id,state,expected_bytes,\
                             checksum_sha256,content_type,correlation_id,created_at_ms,\
                             expires_at_ms,completed_at_ms) \
                             SELECT upload_id,object_key,provider_upload_id,'open',expected_bytes,\
                               checksum_sha256,content_type,correlation_id,created_at_ms,\
                               expires_at_ms,NULL \
                             FROM r2_multipart_creation_claims_v1 \
                             WHERE upload_id=?1 AND claim_token=?2 AND state='provider_bound' \
                               AND provider_upload_id=?3 \
                             ON CONFLICT(upload_id) DO NOTHING",
                        )
                        .bind(&[
                            JsValue::from_str(&claim.upload_id),
                            JsValue::from_str(&claim.claim_token),
                            JsValue::from_str(provider_upload_id),
                        ])
                        .map_err(map_worker_error)?,
                    self.database
                        .prepare(
                            "UPDATE r2_multipart_creation_claims_v1 \
                             SET state='committed',updated_at_ms=?4 \
                             WHERE upload_id=?1 AND claim_token=?2 AND state='provider_bound' \
                               AND provider_upload_id=?3 \
                               AND EXISTS (SELECT 1 FROM r2_multipart_sessions_v1 session \
                                 WHERE session.upload_id=?1 \
                                   AND session.object_key=r2_multipart_creation_claims_v1.object_key \
                                   AND session.provider_upload_id=?3 \
                                   AND session.state='open')",
                        )
                        .bind(&[
                            JsValue::from_str(&claim.upload_id),
                            JsValue::from_str(&claim.claim_token),
                            JsValue::from_str(provider_upload_id),
                            JsValue::from_f64(current.get() as f64),
                        ])
                        .map_err(map_worker_error)?,
                ])
                .into_send()
                .await
                .map_err(map_worker_error)?;
            if results.len() != 2 || results.iter().any(|result| !result.success()) {
                return Err(unavailable());
            }
        } else if claim.state != "committed" {
            return Err(unavailable());
        }
        let committed = self
            .creation_claim(upload_id)
            .await?
            .filter(|stored| {
                stored.state == "committed"
                    && stored.claim_token == claim.claim_token
                    && stored.provider_upload_id.as_deref() == Some(provider_upload_id)
            })
            .ok_or_else(unavailable)?;
        let session = self.session(upload_id).await?.ok_or_else(unavailable)?;
        if committed.upload_id != session.upload_id
            || committed.object_key != session.object_key
            || committed.provider_upload_id.as_deref() != Some(session.provider_upload_id.as_str())
            || committed.expected_bytes != session.expected_bytes
            || committed.checksum_sha256 != session.checksum_sha256
            || committed.content_type != session.content_type
            || committed.correlation_id != session.correlation_id
            || committed.created_at_ms != session._created_at_ms
            || committed.expires_at_ms != session.expires_at_ms
            || session.state != "open"
        {
            return Err(integrity());
        }
        Ok(session)
    }

    async fn reconcile_provider_creation(
        &self,
        context: StorageRequestContext,
        request: &ProviderCreateMultipartRequestV1,
        claim: &CreationClaimRow,
        current: TimestampMillis,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        if !Self::exact_creation_claim(claim, context, request)? {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        let session = self.commit_provider_creation_claim(claim, current).await?;
        let geometry = self.intent_geometry(request.upload_id()).await?;
        if !Self::exact_create_replay(&session, context, request, &geometry)? {
            return Err(integrity());
        }
        Self::provider_session(&session)
    }

    fn exact_create_replay(
        session: &SessionRow,
        context: StorageRequestContext,
        request: &ProviderCreateMultipartRequestV1,
        geometry: &MultipartIntentGeometryRow,
    ) -> Result<bool, StorageFailure> {
        Ok(session.object_key == request.key().as_str()
            && session.expected_bytes == signed(request.spec().total_size().get())?
            && session.checksum_sha256 == request.spec().checksum_sha256().as_str()
            && session.content_type == request.spec().content_type().as_str()
            && session.expires_at_ms == request.expires_at().get()
            && session.correlation_id == context.correlation_id().to_string()
            && geometry.part_size == signed(request.spec().part_size().get())?
            && geometry.part_count == i64::from(request.spec().part_count())
            && matches!(session.state.as_str(), "open" | "completing"))
    }

    fn validate_part_geometry(
        session: &SessionRow,
        geometry: &MultipartIntentGeometryRow,
        part_number: MultipartPartNumberV1,
        size: u64,
    ) -> Result<(), StorageFailure> {
        let part_count = u16::try_from(geometry.part_count).map_err(|_| integrity())?;
        let part_size = u64::try_from(geometry.part_size).map_err(|_| integrity())?;
        let total_size = u64::try_from(session.expected_bytes).map_err(|_| integrity())?;
        if part_count == 0 || part_number.get() > part_count || part_size == 0 {
            return Err(integrity());
        }
        let expected = if part_number.get() < part_count {
            part_size
        } else {
            u64::from(part_count - 1)
                .checked_mul(part_size)
                .and_then(|preceding| total_size.checked_sub(preceding))
                .filter(|remaining| *remaining > 0 && *remaining <= part_size)
                .ok_or_else(integrity)?
        };
        if size == expected {
            Ok(())
        } else {
            Err(invalid())
        }
    }

    fn validate_completion_geometry(
        session: &SessionRow,
        geometry: &MultipartIntentGeometryRow,
        parts: &[ProviderPartReceiptV1],
    ) -> Result<(), StorageFailure> {
        let expected_count = usize::try_from(geometry.part_count).map_err(|_| integrity())?;
        if parts.len() != expected_count {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        let mut total = 0_u64;
        for (index, part) in parts.iter().enumerate() {
            let expected_number = u16::try_from(index + 1).map_err(|_| integrity())?;
            if part.part_number().get() != expected_number {
                return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
            }
            Self::validate_part_geometry(session, geometry, part.part_number(), part.size().get())?;
            total = total.checked_add(part.size().get()).ok_or_else(integrity)?;
        }
        if total == u64::try_from(session.expected_bytes).map_err(|_| integrity())? {
            Ok(())
        } else {
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        }
    }

    async fn claim_part(
        &self,
        session: &SessionRow,
        request: &ProviderPutPartRequestV1,
        current: TimestampMillis,
    ) -> Result<String, StorageFailure> {
        let claim_token = CorrelationId::new().to_string();
        let lease_expires_at_ms = current
            .get()
            .saturating_add(PART_CLAIM_LEASE_MS)
            .min(session.expires_at_ms);
        if lease_expires_at_ms <= current.get() {
            return Err(invalid());
        }
        let claimed = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_part_claims_v1(\
                 upload_id,part_number,bytes,checksum_sha256,claim_token,claimed_at_ms,lease_expires_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(upload_id,part_number) DO UPDATE SET \
                   claim_token=excluded.claim_token,claimed_at_ms=excluded.claimed_at_ms,\
                   lease_expires_at_ms=excluded.lease_expires_at_ms \
                 WHERE r2_multipart_part_claims_v1.bytes=excluded.bytes \
                   AND r2_multipart_part_claims_v1.checksum_sha256=excluded.checksum_sha256 \
                   AND r2_multipart_part_claims_v1.lease_expires_at_ms<=excluded.claimed_at_ms \
                 RETURNING claim_token",
            )
            .bind(&[
                JsValue::from_str(&request.reference().upload_id().to_string()),
                JsValue::from_f64(f64::from(request.part_number().get())),
                number(u64::try_from(request.bytes().len()).map_err(|_| invalid())?)?,
                JsValue::from_str(request.checksum_sha256().as_str()),
                JsValue::from_str(&claim_token),
                JsValue::from_f64(current.get() as f64),
                JsValue::from_f64(lease_expires_at_ms as f64),
            ])
            .map_err(map_worker_error)?
            .first::<PartClaimTokenRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if claimed.is_some_and(|row| row.claim_token == claim_token) {
            return Ok(claim_token);
        }
        let existing = self
            .database
            .prepare(
                "SELECT bytes,checksum_sha256 FROM r2_multipart_part_claims_v1 \
                 WHERE upload_id=?1 AND part_number=?2 LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&request.reference().upload_id().to_string()),
                JsValue::from_f64(f64::from(request.part_number().get())),
            ])
            .map_err(map_worker_error)?
            .first::<PartClaimShapeRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(unavailable)?;
        if existing.bytes != signed(u64::try_from(request.bytes().len()).map_err(|_| invalid())?)?
            || existing.checksum_sha256 != request.checksum_sha256().as_str()
        {
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        } else {
            Err(unavailable())
        }
    }

    async fn claim_completion(
        &self,
        upload_id: MultipartUploadId,
        request_parts_sha256: &str,
        current: TimestampMillis,
    ) -> Result<String, StorageFailure> {
        let claim_token = CorrelationId::new().to_string();
        let lease_expires_at_ms = current
            .get()
            .checked_add(COMPLETION_CLAIM_LEASE_MS)
            .filter(|value| {
                *value <= i64::try_from(frame_domain::MAX_WIRE_INTEGER).unwrap_or(i64::MAX)
            })
            .ok_or_else(integrity)?;
        let claimed = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_completion_claims_v1(\
                 upload_id,request_parts_sha256,claim_token,state,attempt_count,claimed_at_ms,\
                 lease_expires_at_ms,completed_at_ms) \
                 VALUES(?1,?2,?3,'active',1,?4,?5,NULL) \
                 ON CONFLICT(upload_id) DO UPDATE SET \
                   claim_token=excluded.claim_token,\
                   attempt_count=r2_multipart_completion_claims_v1.attempt_count+1,\
                   claimed_at_ms=excluded.claimed_at_ms,\
                   lease_expires_at_ms=excluded.lease_expires_at_ms \
                 WHERE r2_multipart_completion_claims_v1.state='active' \
                   AND r2_multipart_completion_claims_v1.request_parts_sha256=excluded.request_parts_sha256 \
                   AND r2_multipart_completion_claims_v1.lease_expires_at_ms<=excluded.claimed_at_ms \
                 RETURNING claim_token",
            )
            .bind(&[
                JsValue::from_str(&upload_id.to_string()),
                JsValue::from_str(request_parts_sha256),
                JsValue::from_str(&claim_token),
                JsValue::from_f64(current.get() as f64),
                JsValue::from_f64(lease_expires_at_ms as f64),
            ])
            .map_err(map_worker_error)?
            .first::<PartClaimTokenRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if claimed.is_some_and(|row| row.claim_token == claim_token) {
            return Ok(claim_token);
        }
        let existing = self
            .database
            .prepare(
                "SELECT request_parts_sha256,claim_token,state,attempt_count,lease_expires_at_ms \
                 FROM r2_multipart_completion_claims_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_id.to_string())])
            .map_err(map_worker_error)?
            .first::<CompletionClaimShapeRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(unavailable)?;
        if existing.request_parts_sha256 != request_parts_sha256 {
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        } else if existing.state == "active"
            && existing.attempt_count > 0
            && existing.lease_expires_at_ms > current.get()
            && existing.claim_token != claim_token
        {
            Err(unavailable())
        } else {
            Err(integrity())
        }
    }

    async fn parts(
        &self,
        session: &SessionRow,
        correlation: CorrelationId,
    ) -> Result<Vec<ProviderPartReceiptV1>, StorageFailure> {
        let key = session.key()?;
        let rows = self
            .database
            .prepare(
                "SELECT part_number,bytes,checksum_sha256,provider_etag,uploaded_at_ms \
                 FROM r2_multipart_parts_v1 WHERE upload_id=?1 ORDER BY part_number",
            )
            .bind(&[JsValue::from_str(&session.upload_id)])
            .map_err(map_worker_error)?
            .all()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .results::<PartRow>()
            .map_err(map_worker_error)?;
        rows.into_iter()
            .map(|row| row.receipt(session.upload()?, key.clone(), correlation))
            .collect()
    }

    fn validate_reference(
        context: StorageRequestContext,
        reference: &ProviderUploadReferenceV1,
        session: &SessionRow,
    ) -> Result<(), StorageFailure> {
        Self::authorize(context, reference.key())?;
        if reference.upload_id() != session.upload()?
            || reference.key().as_str() != session.object_key
            || reference.handle().expose_for_provider()
                != encode_handle(&session.provider_upload_id)
            || reference.correlation_id() != context.correlation_id()
            || session.correlation_id != context.correlation_id().to_string()
        {
            return Err(StorageFailure::new(StorageFailureKind::Unauthorized));
        }
        Ok(())
    }

    fn provider_session(
        session: &SessionRow,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        Ok(ProviderMultipartSessionV1::new(
            session.upload()?,
            session.key()?,
            ProviderMultipartHandleV1::parse(encode_handle(&session.provider_upload_id))?,
            timestamp(session.expires_at_ms)?,
            correlation(&session.correlation_id)?,
        ))
    }

    /// Reconstructs the server-only provider reference for an authenticated
    /// route. The opaque R2 upload handle never crosses the API boundary.
    pub async fn route_reference(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
        key: &ScopedObjectKey,
    ) -> Result<ProviderUploadReferenceV1, StorageFailure> {
        Self::authorize(context, key)?;
        let session = self.session(upload_id).await?.ok_or_else(not_found)?;
        if session.object_key != key.as_str()
            || session.correlation_id != context.correlation_id().to_string()
        {
            return Err(StorageFailure::new(StorageFailureKind::Unauthorized));
        }
        Ok(ProviderUploadReferenceV1::new(
            upload_id,
            key.clone(),
            ProviderMultipartHandleV1::parse(encode_handle(&session.provider_upload_id))?,
            context.correlation_id(),
        ))
    }

    async fn completion(
        &self,
        request: &ProviderCompleteMultipartRequestV1,
        correlation_id: CorrelationId,
    ) -> Result<Option<ProviderCompletedObjectV1>, StorageFailure> {
        let reference = request.reference();
        let row = self
            .database
            .prepare(
                "SELECT c.request_parts_sha256,c.provider_version,c.provider_etag,c.bytes,c.checksum_sha256,c.content_type,\
                 c.container,c.video_codec,c.audio_codec,c.width,c.height,c.duration_ms,\
                 c.frame_rate_millihertz,c.completed_at_ms,c.correlation_id \
                 FROM r2_multipart_completions_v1 c WHERE c.upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&reference.upload_id().to_string())])
            .map_err(map_worker_error)?
            .first::<CompletionRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.request_parts_sha256 != completion_parts_digest(request.parts())
            || row.bytes != signed(request.expected_size().get())?
            || row.checksum_sha256 != request.expected_checksum_sha256().as_str()
            || row.content_type != request.expected_content_type().as_str()
            || row.correlation_id != correlation_id.to_string()
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        row.completed(
            reference.upload_id(),
            reference.key().clone(),
            correlation_id,
        )
        .map(Some)
    }

    async fn verify_full_object(
        &self,
        key: &ScopedObjectKey,
        expected_size: ByteSize,
        expected_checksum: &ChecksumSha256,
        expected_content_type: &ContentType,
    ) -> Result<worker::Object, StorageFailure> {
        self.verify_full_object_local(key, expected_size, expected_checksum, expected_content_type)
            .into_send()
            .await
    }

    async fn verify_full_object_local(
        &self,
        key: &ScopedObjectKey,
        expected_size: ByteSize,
        expected_checksum: &ChecksumSha256,
        expected_content_type: &ContentType,
    ) -> Result<worker::Object, StorageFailure> {
        let object = self
            .bucket
            .get(key.as_str())
            .execute()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(not_found)?;
        let http = object.http_metadata();
        if object.size() != expected_size.get()
            || http.content_type.as_deref() != Some(expected_content_type.as_str())
            || http.content_encoding.is_some()
        {
            return Err(integrity());
        }
        let mut body = object
            .body()
            .ok_or_else(integrity)?
            .stream()
            .map_err(map_worker_error)?;
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        while let Some(chunk) = body.try_next().await.map_err(map_worker_error)? {
            count = count
                .checked_add(u64::try_from(chunk.len()).map_err(|_| integrity())?)
                .ok_or_else(integrity)?;
            if count > expected_size.get() {
                return Err(integrity());
            }
            digest.update(chunk);
        }
        if count != expected_size.get() || hex(&digest.finalize()) != expected_checksum.as_str() {
            return Err(integrity());
        }
        Ok(object)
    }

    async fn persist_verified_object(
        &self,
        request: &ProviderCompleteMultipartRequestV1,
        object: &worker::Object,
        verified_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let upload_id = request.reference().upload_id().to_string();
        let inserted = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_verified_objects_v1(\
                 upload_id,provider_version,provider_etag,bytes,checksum_sha256,content_type,verified_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(upload_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&upload_id),
                JsValue::from_str(&object.version()),
                JsValue::from_str(&object.etag()),
                number(request.expected_size().get())?,
                JsValue::from_str(request.expected_checksum_sha256().as_str()),
                JsValue::from_str(request.expected_content_type().as_str()),
                JsValue::from_f64(verified_at.get() as f64),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !inserted.success() {
            return Err(unavailable());
        }
        let stored = self
            .database
            .prepare(
                "SELECT provider_version,provider_etag,bytes,checksum_sha256,content_type \
                 FROM r2_multipart_verified_objects_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&upload_id)])
            .map_err(map_worker_error)?
            .first::<VerifiedObjectRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(unavailable)?;
        if stored.provider_version != object.version()
            || stored.provider_etag != object.etag()
            || stored.bytes != signed(request.expected_size().get())?
            || stored.checksum_sha256 != request.expected_checksum_sha256().as_str()
            || stored.content_type != request.expected_content_type().as_str()
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        Ok(())
    }

    async fn persist_completion(
        &self,
        request: &ProviderCompleteMultipartRequestV1,
        object: &worker::Object,
        probe: &TrustedMediaProbeV1,
        completion_claim_token: &str,
        completed_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let reference = request.reference();
        let parts_digest = completion_parts_digest(request.parts());
        let statements = vec![
            self.database
                .prepare(
                    "INSERT INTO r2_multipart_completions_v1(\
                     upload_id,request_parts_sha256,provider_version,provider_etag,bytes,checksum_sha256,content_type,\
                     container,video_codec,audio_codec,width,height,duration_ms,frame_rate_millihertz,\
                     completed_at_ms,correlation_id,completion_claim_token) \
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) \
                     ON CONFLICT(upload_id) DO NOTHING",
                )
                .bind(&[
                    JsValue::from_str(&reference.upload_id().to_string()),
                    JsValue::from_str(&parts_digest),
                    JsValue::from_str(&object.version()),
                    JsValue::from_str(&object.etag()),
                    number(request.expected_size().get())?,
                    JsValue::from_str(request.expected_checksum_sha256().as_str()),
                    JsValue::from_str(request.expected_content_type().as_str()),
                    JsValue::from_str(container_name(probe.container())),
                    JsValue::from_str(video_codec_name(probe.video_codec())),
                    JsValue::from_str(audio_codec_name(probe.audio_codec())),
                    JsValue::from_f64(f64::from(probe.width())),
                    JsValue::from_f64(f64::from(probe.height())),
                    number(probe.duration_ms())?,
                    JsValue::from_f64(f64::from(probe.frame_rate_millihertz())),
                    JsValue::from_f64(completed_at.get() as f64),
                    JsValue::from_str(&reference.correlation_id().to_string()),
                    JsValue::from_str(completion_claim_token),
                ])
                .map_err(map_worker_error)?,
            self.database
                .prepare(
                    "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=?2 \
                     WHERE upload_id=?1 AND state='completing' \
                       AND EXISTS (SELECT 1 FROM r2_multipart_completions_v1 c \
                         WHERE c.upload_id=?1 AND c.request_parts_sha256=?3 AND c.bytes=?4 \
                           AND c.checksum_sha256=?5 AND c.content_type=?6 AND c.correlation_id=?7 \
                           AND c.completion_claim_token=?8)",
                )
                .bind(&[
                    JsValue::from_str(&reference.upload_id().to_string()),
                    JsValue::from_f64(completed_at.get() as f64),
                    JsValue::from_str(&parts_digest),
                    number(request.expected_size().get())?,
                    JsValue::from_str(request.expected_checksum_sha256().as_str()),
                    JsValue::from_str(request.expected_content_type().as_str()),
                    JsValue::from_str(&reference.correlation_id().to_string()),
                    JsValue::from_str(completion_claim_token),
                ])
                .map_err(map_worker_error)?,
            self.database
                .prepare(
                    "UPDATE r2_multipart_completion_claims_v1 \
                     SET state='complete',completed_at_ms=?4 \
                     WHERE upload_id=?1 AND claim_token=?2 AND state='active' \
                       AND request_parts_sha256=?3 \
                       AND EXISTS (SELECT 1 FROM r2_multipart_completions_v1 completion \
                         WHERE completion.upload_id=?1 \
                           AND completion.request_parts_sha256=?3 \
                           AND completion.completion_claim_token=?2)",
                )
                .bind(&[
                    JsValue::from_str(&reference.upload_id().to_string()),
                    JsValue::from_str(completion_claim_token),
                    JsValue::from_str(&parts_digest),
                    JsValue::from_f64(completed_at.get() as f64),
                ])
                .map_err(map_worker_error)?,
        ];
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if results.len() != 3
            || results.iter().any(|result| !result.success())
            || results
                .iter()
                .any(|result| result.meta().ok().flatten().and_then(|meta| meta.changes) != Some(1))
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Starts or resumes the provider side of an authenticated abort. The
    /// pending reconciliation is committed before the R2 call, while the
    /// caller owns the authority-fenced D1 terminal batch that also updates
    /// the product upload row.
    pub async fn reconcile_authenticated_abort_provider(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
        attempt: i64,
        now: TimestampMillis,
    ) -> Result<AuthenticatedAbortOutcomeV1, StorageFailure> {
        self.require_provider_mutations_enabled().await?;
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, &session)?;
        match session.state.as_str() {
            "complete" => return Ok(AuthenticatedAbortOutcomeV1::AlreadyCompleted),
            "aborted" | "expired" => {
                return Ok(AuthenticatedAbortOutcomeV1::AlreadyAborted);
            }
            "open" | "completing" => {}
            _ => return Err(integrity()),
        }
        if !self
            .abort_attempt_is_current(
                &session.upload_id,
                AbortIntentKind::AuthenticatedDelete,
                attempt,
                now,
            )
            .await?
        {
            return Ok(AuthenticatedAbortOutcomeV1::Pending);
        }
        match self.attempt_provider_abort(&session).await? {
            AbortProviderAttempt::Terminal(AbortTerminalOutcome::Confirmed) => {
                Ok(AuthenticatedAbortOutcomeV1::Confirmed { attempt })
            }
            AbortProviderAttempt::Terminal(AbortTerminalOutcome::PreservedObject) => {
                Ok(AuthenticatedAbortOutcomeV1::PreservedObject { attempt })
            }
            AbortProviderAttempt::RetryableFailure(failure) => Err(failure),
        }
    }

    pub async fn cleanup_stale(
        &self,
        now: TimestampMillis,
        limit: u16,
    ) -> Result<u16, StorageFailure> {
        if !(1..=100).contains(&limit) {
            return Err(invalid());
        }
        if !self.provider_mutations_enabled().await? {
            return Ok(0);
        }
        let bound_claims = self
            .database
            .prepare(
                "SELECT upload_id,organization_id,object_key,expected_bytes,checksum_sha256,\
                 content_type,correlation_id,part_size,part_count,expires_at_ms,claim_token,state,\
                 provider_upload_id,created_at_ms \
                 FROM r2_multipart_creation_claims_v1 \
                 WHERE state='provider_bound' AND expires_at_ms<=?1 \
                 ORDER BY expires_at_ms,upload_id LIMIT ?2",
            )
            .bind(&[
                JsValue::from_f64(now.get() as f64),
                JsValue::from_f64(f64::from(limit)),
            ])
            .map_err(map_worker_error)?
            .all()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .results::<CreationClaimRow>()
            .map_err(map_worker_error)?;
        for claim in bound_claims {
            self.commit_provider_creation_claim(&claim, now).await?;
        }
        let rows = self
            .database
            .prepare(
                "SELECT session.upload_id,session.object_key,session.provider_upload_id,\
                 session.state,session.expected_bytes,session.checksum_sha256,\
                 session.content_type,session.correlation_id,session.created_at_ms,\
                 session.expires_at_ms,session.completed_at_ms \
                 FROM r2_multipart_sessions_v1 session \
                 LEFT JOIN r2_multipart_abort_reconciliation_v1 reconciliation \
                   ON reconciliation.upload_id=session.upload_id \
                 WHERE session.state='open' AND session.expires_at_ms<=?1 \
                   AND (reconciliation.upload_id IS NULL OR (reconciliation.state='pending' \
                     AND reconciliation.intent_kind='expiry_cleanup' \
                     AND reconciliation.next_attempt_at_ms<=?1)) \
                 ORDER BY session.expires_at_ms,session.upload_id LIMIT ?2",
            )
            .bind(&[
                JsValue::from_f64(now.get() as f64),
                JsValue::from_f64(f64::from(limit)),
            ])
            .map_err(map_worker_error)?
            .all()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .results::<SessionRow>()
            .map_err(map_worker_error)?;
        let mut cleaned = 0_u16;
        for row in rows {
            let Some(attempt) = self
                .begin_abort_attempt(&row, AbortIntentKind::ExpiryCleanup, now)
                .await?
            else {
                continue;
            };
            match self.attempt_provider_abort(&row).await? {
                AbortProviderAttempt::Terminal(outcome) => {
                    self.finish_abort_reconciliation(
                        &row.upload_id,
                        AbortIntentKind::ExpiryCleanup,
                        attempt,
                        outcome,
                        now,
                    )
                    .await?;
                    cleaned = cleaned.saturating_add(1);
                }
                AbortProviderAttempt::RetryableFailure(failure) => {
                    self.retain_abort_failure(
                        &row.upload_id,
                        AbortIntentKind::ExpiryCleanup,
                        attempt,
                        &failure,
                        now,
                    )
                    .await?;
                }
            }
        }
        Ok(cleaned)
    }

    async fn attempt_provider_abort(
        &self,
        session: &SessionRow,
    ) -> Result<AbortProviderAttempt, StorageFailure> {
        let head = match self.bucket.head(&session.object_key).into_send().await {
            Ok(object) => object,
            Err(error) => {
                let failure = map_worker_error(error);
                if failure.kind() == StorageFailureKind::NotFound {
                    None
                } else {
                    return Ok(AbortProviderAttempt::RetryableFailure(failure));
                }
            }
        };
        if head.is_some() {
            return Ok(AbortProviderAttempt::Terminal(
                AbortTerminalOutcome::PreservedObject,
            ));
        }
        let abort_result = match self
            .bucket
            .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
        {
            Ok(upload) => upload.abort().into_send().await.map_err(map_worker_error),
            Err(error) => Err(map_worker_error(error)),
        };
        match abort_result {
            Ok(()) => Ok(AbortProviderAttempt::Terminal(
                AbortTerminalOutcome::Confirmed,
            )),
            Err(failure) if failure.kind() == StorageFailureKind::NotFound => Ok(
                AbortProviderAttempt::Terminal(AbortTerminalOutcome::Confirmed),
            ),
            Err(failure) => Ok(AbortProviderAttempt::RetryableFailure(failure)),
        }
    }

    async fn abort_attempt_is_current(
        &self,
        upload_id: &str,
        intent: AbortIntentKind,
        attempt: i64,
        now: TimestampMillis,
    ) -> Result<bool, StorageFailure> {
        let stored = self
            .database
            .prepare(
                "SELECT intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class \
                 FROM r2_multipart_abort_reconciliation_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(upload_id)])
            .map_err(map_worker_error)?
            .first::<AbortReconciliationRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        Ok(stored.is_some_and(|row| {
            row.intent_kind == intent.as_str()
                && row.state == "pending"
                && row.attempt_count == attempt
                && row.next_attempt_at_ms > now.get()
                && row.last_failure_class.is_none()
        }))
    }

    async fn begin_abort_attempt(
        &self,
        session: &SessionRow,
        intent: AbortIntentKind,
        now: TimestampMillis,
    ) -> Result<Option<i64>, StorageFailure> {
        let lock_until = abort_attempt_lock_until(now.get());
        let result = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_abort_reconciliation_v1(\
                 upload_id,intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class,\
                 started_at_ms,updated_at_ms,terminal_at_ms) \
                 SELECT ?1,?4,'pending',1,?2,NULL,?3,?3,NULL \
                 WHERE EXISTS (SELECT 1 FROM r2_multipart_sessions_v1 session \
                   WHERE session.upload_id=?1 AND session.state='open' \
                     AND (?4!='expiry_cleanup' OR session.expires_at_ms<=?3)) \
                 ON CONFLICT(upload_id) DO UPDATE SET \
                   attempt_count=attempt_count+1,next_attempt_at_ms=?2,\
                   last_failure_class=NULL,updated_at_ms=?3 \
                 WHERE state='pending' AND intent_kind=?4 AND next_attempt_at_ms<=?3",
            )
            .bind(&[
                JsValue::from_str(&session.upload_id),
                JsValue::from_f64(lock_until as f64),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(intent.as_str()),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !result.success() {
            return Err(unavailable());
        }
        if result
            .meta()
            .map_err(map_worker_error)?
            .and_then(|meta| meta.changes)
            != Some(1)
        {
            return Ok(None);
        }
        let stored = self
            .database
            .prepare(
                "SELECT intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class \
                 FROM r2_multipart_abort_reconciliation_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(&session.upload_id)])
            .map_err(map_worker_error)?
            .first::<AbortReconciliationRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        Ok(stored
            .filter(|row| {
                row.intent_kind == intent.as_str()
                    && row.state == "pending"
                    && row.next_attempt_at_ms == lock_until
                    && row.last_failure_class.is_none()
            })
            .map(|row| row.attempt_count))
    }

    async fn retain_abort_failure(
        &self,
        upload_id: &str,
        intent: AbortIntentKind,
        attempt: i64,
        failure: &StorageFailure,
        now: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let class = abort_failure_class(failure.kind());
        let next_attempt = abort_retry_at(now.get(), attempt);
        let result = self
            .database
            .prepare(
                "UPDATE r2_multipart_abort_reconciliation_v1 SET next_attempt_at_ms=?3,\
                 last_failure_class=?4,updated_at_ms=?5 \
                 WHERE upload_id=?1 AND intent_kind=?6 AND state='pending' AND attempt_count=?2",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(attempt as f64),
                JsValue::from_f64(next_attempt as f64),
                JsValue::from_str(class),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(intent.as_str()),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !result.success() {
            return Err(unavailable());
        }
        let stored = self
            .database
            .prepare(
                "SELECT intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class \
                 FROM r2_multipart_abort_reconciliation_v1 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(upload_id)])
            .map_err(map_worker_error)?
            .first::<AbortReconciliationRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if stored.is_some_and(|row| {
            row.intent_kind == intent.as_str()
                && row.state == "pending"
                && row.attempt_count == attempt
                && row.next_attempt_at_ms == next_attempt
                && row.last_failure_class.as_deref() == Some(class)
        }) {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    async fn finish_abort_reconciliation(
        &self,
        upload_id: &str,
        intent: AbortIntentKind,
        attempt: i64,
        outcome: AbortTerminalOutcome,
        now: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let operation_id = uuid::Uuid::now_v7().to_string();
        let mut statements = Vec::with_capacity(9);
        statements.push(
            self.database
                .prepare(
                    "UPDATE r2_multipart_sessions_v1 SET state=?2 \
                     WHERE upload_id=?1 AND ((?3='expiry_cleanup' AND state='open' \
                       AND expires_at_ms<=?4) OR (?3='authenticated_delete' \
                       AND state IN ('open','completing')))",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_str(outcome.session_state(intent)),
                    JsValue::from_str(intent.as_str()),
                    JsValue::from_f64(now.get() as f64),
                ])
                .map_err(map_worker_error)?,
        );
        statements.push(abort_change_assertion(
            self.database,
            &operation_id,
            upload_id,
            "session_transition",
        )?);
        if intent == AbortIntentKind::AuthenticatedDelete
            && outcome == AbortTerminalOutcome::Confirmed
        {
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(format!("frame.multipart.abort.v1\0{upload_id}").as_bytes())
            );
            statements.push(
                self.database
                    .prepare(
                        "UPDATE video_uploads SET state='aborted',updated_at_ms=?2,\
                         revision=revision+1,event_sequence=event_sequence+1,event_fingerprint=?3 \
                         WHERE id=?1 AND state IN ('initiated','uploading','finalizing','failed')",
                    )
                    .bind(&[
                        JsValue::from_str(upload_id),
                        JsValue::from_f64(now.get() as f64),
                        JsValue::from_str(&fingerprint),
                    ])
                    .map_err(map_worker_error)?,
            );
            statements.push(abort_change_assertion(
                self.database,
                &operation_id,
                upload_id,
                "video_upload_transition",
            )?);
        }
        statements.push(
            self.database
                .prepare(
                    "UPDATE r2_multipart_abort_reconciliation_v1 SET state=?3,\
                     next_attempt_at_ms=?4,last_failure_class=NULL,updated_at_ms=?4,terminal_at_ms=?4 \
                     WHERE upload_id=?1 AND intent_kind=?5 AND state='pending' AND attempt_count=?2",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_f64(attempt as f64),
                    JsValue::from_str(outcome.reconciliation_state()),
                    JsValue::from_f64(now.get() as f64),
                    JsValue::from_str(intent.as_str()),
                ])
                .map_err(map_worker_error)?,
        );
        statements.push(abort_change_assertion(
            self.database,
            &operation_id,
            upload_id,
            "reconciliation_transition",
        )?);
        statements.push(
            self.database
                .prepare(
                    "INSERT INTO r2_multipart_abort_terminal_assertions_v1(\
                     upload_id,outcome,asserted_at_ms) VALUES(?1,?2,?3)",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_str(outcome.reconciliation_state()),
                    JsValue::from_f64(now.get() as f64),
                ])
                .map_err(map_worker_error)?,
        );
        statements.push(abort_change_assertion(
            self.database,
            &operation_id,
            upload_id,
            "terminal_assertion",
        )?);
        statements.push(
            self.database
                .prepare("DELETE FROM r2_multipart_abort_batch_assertions_v1 WHERE operation_id=?1")
                .bind(&[JsValue::from_str(&operation_id)])
                .map_err(map_worker_error)?,
        );
        let expected_results = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if results.len() == expected_results && results.iter().all(worker::D1Result::success) {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    fn download_metadata(
        object: &ProviderCompletedObjectV1,
        correlation: CorrelationId,
    ) -> ProviderDownloadMetadataV1 {
        ProviderDownloadMetadataV1::new(
            object.key().clone(),
            object.size(),
            object.checksum_sha256().clone(),
            object.content_type().clone(),
            object.provider_version().clone(),
            object.provider_etag().clone(),
            object.last_modified(),
            correlation,
        )
    }

    fn condition(
        validator: &DownloadValidatorV1,
        metadata: ProviderDownloadMetadataV1,
    ) -> Result<Option<ProviderDownloadResponseV1>, StorageFailure> {
        match validator {
            DownloadValidatorV1::None => Ok(None),
            DownloadValidatorV1::IfMatch(expected) if expected != metadata.provider_etag() => {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            }
            DownloadValidatorV1::IfNoneMatch(expected) if expected == metadata.provider_etag() => {
                Ok(Some(ProviderDownloadResponseV1::NotModified(metadata)))
            }
            DownloadValidatorV1::IfMatch(_) | DownloadValidatorV1::IfNoneMatch(_) => Ok(None),
        }
    }
}

#[async_trait]
impl<P: TrustedR2MediaProbeV1 + ?Sized> MultipartObjectStoreV1 for R2MultipartObjectStoreV1<'_, P> {
    fn capabilities(&self) -> MultipartProviderCapabilitiesV1 {
        self.capabilities
    }

    async fn create_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCreateMultipartRequestV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        self.require_provider_mutations_enabled().await?;
        let created_at = now()?;
        if request.correlation_id() != context.correlation_id()
            || request.spec().total_size() > self.capabilities.max_total_size()
            || request.spec().part_size() < self.capabilities.min_part_size()
            || request.spec().part_size() > self.capabilities.max_part_size()
            || request.spec().part_count() > self.capabilities.max_part_count()
        {
            return Err(invalid());
        }
        if request.expires_at() <= created_at {
            if let Some(claim) = self.creation_claim(request.upload_id()).await? {
                if claim.organization_id != context.tenant_id().to_string() {
                    return Err(StorageFailure::new(StorageFailureKind::Unauthorized));
                }
                if !Self::exact_creation_claim(&claim, context, &request)? {
                    return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
                }
                // A provider handle committed before expiry must never remain
                // invisible to stale cleanup merely because the Worker lost
                // the following D1 session response.
                if claim.state == "provider_bound" {
                    self.reconcile_provider_creation(context, &request, &claim, created_at)
                        .await?;
                }
            }
            return Err(invalid());
        }
        if request.expires_at().get().saturating_sub(created_at.get()) > MAX_SESSION_TTL_MS {
            return Err(invalid());
        }
        if let Some(existing) = self.session(request.upload_id()).await? {
            let geometry = self.intent_geometry(request.upload_id()).await?;
            return if Self::exact_create_replay(&existing, context, &request, &geometry)? {
                Self::provider_session(&existing)
            } else {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            };
        }
        let authorized = self
            .database
            .prepare(
                "SELECT 1 AS present FROM video_uploads upload \
                 JOIN r2_multipart_intents_v1 intent ON intent.upload_id=upload.id \
                 JOIN storage_integrations integration ON integration.id=intent.integration_id \
                 WHERE upload.id=?1 AND upload.organization_id=?2 \
                   AND upload.source_object_key=?3 AND upload.expected_bytes=?4 \
                   AND upload.content_type=?5 AND upload.transfer_mode='brokered' \
                   AND upload.state IN ('initiated','uploading') \
                   AND intent.checksum_sha256=?6 AND intent.part_size=?7 \
                   AND intent.part_count=?8 AND intent.expires_at_ms=?9 \
                   AND integration.organization_id=upload.organization_id \
                   AND integration.provider='r2' AND integration.state='active' \
                   AND json_extract(integration.capabilities_json,'$.multipart')=1 LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&request.upload_id().to_string()),
                JsValue::from_str(&context.tenant_id().to_string()),
                JsValue::from_str(request.key().as_str()),
                number(request.spec().total_size().get())?,
                JsValue::from_str(request.spec().content_type().as_str()),
                JsValue::from_str(request.spec().checksum_sha256().as_str()),
                number(request.spec().part_size().get())?,
                JsValue::from_f64(f64::from(request.spec().part_count())),
                JsValue::from_f64(request.expires_at().get() as f64),
            ])
            .map_err(map_worker_error)?
            .first::<PresenceRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_some_and(|row| row.present == 1);
        if !authorized {
            return Err(StorageFailure::new(StorageFailureKind::Unauthorized));
        }
        let reservation = self
            .reserve_provider_creation(context, &request, created_at)
            .await?;
        if !reservation.acquired {
            let reconciled = match reservation.claim.state.as_str() {
                "provider_bound" | "committed" => {
                    self.reconcile_provider_creation(
                        context,
                        &request,
                        &reservation.claim,
                        created_at,
                    )
                    .await
                }
                "reserved" => Err(unavailable()),
                _ => Err(integrity()),
            }?;
            return if request.expires_at() <= now()? {
                Err(invalid())
            } else {
                Ok(reconciled)
            };
        }
        let metadata = HttpMetadata {
            content_type: Some(request.spec().content_type().as_str().into()),
            content_disposition: Some("attachment".into()),
            cache_control: Some("private, no-store".into()),
            ..HttpMetadata::default()
        };
        // R2 multipart creation has no idempotency key and cannot commit with
        // D1 atomically. If this Worker dies after `execute` returns but before
        // `bind_provider_creation` commits, the reserved D1 claim deliberately
        // blocks automatic retries; the metadata claim token is the operator's
        // authority for manual orphan reconciliation.
        let upload = self
            .bucket
            .create_multipart_upload(request.key().as_str())
            .http_metadata(metadata)
            .custom_metadata(std::collections::HashMap::from([
                (
                    "frame-correlation-id".into(),
                    context.correlation_id().to_string(),
                ),
                ("frame-cache-policy".into(), "no_store".into()),
                (
                    "frame-sha256".into(),
                    request.spec().checksum_sha256().as_str().into(),
                ),
                (
                    "frame-creation-claim".into(),
                    reservation.claim.claim_token.clone(),
                ),
            ]))
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let provider_id = upload.upload_id().into_send().await;
        ProviderMultipartHandleV1::parse(encode_handle(&provider_id))?;
        let bound = match self
            .bind_provider_creation(
                request.upload_id(),
                &reservation.claim.claim_token,
                &provider_id,
                now()?,
            )
            .await
        {
            Ok(claim) => claim,
            Err(binding_failure) => match upload.abort().into_send().await {
                Ok(()) => return Err(binding_failure),
                Err(abort_error) => self
                    .bind_provider_creation(
                        request.upload_id(),
                        &reservation.claim.claim_token,
                        &provider_id,
                        now()?,
                    )
                    .await
                    .map_err(|_| map_worker_error(abort_error))?,
            },
        };
        let reconciled = self
            .reconcile_provider_creation(context, &request, &bound, now()?)
            .await?;
        if request.expires_at() <= now()? {
            Err(invalid())
        } else {
            Ok(reconciled)
        }
    }

    async fn lookup_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderLookupMultipartRequestV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.correlation_id() != context.correlation_id() {
            return Err(invalid());
        }
        let Some(session) = self.session(request.upload_id()).await? else {
            return Ok(None);
        };
        if session.object_key != request.key().as_str()
            || session.correlation_id != context.correlation_id().to_string()
        {
            return Err(StorageFailure::new(StorageFailureKind::Unauthorized));
        }
        if matches!(session.state.as_str(), "aborted" | "expired" | "complete")
            || (session.state == "open" && now()?.get() >= session.expires_at_ms)
        {
            return Ok(None);
        }
        Self::provider_session(&session).map(Some)
    }

    async fn list_parts(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderPartsListV1, StorageFailure> {
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, &session)?;
        if !matches!(session.state.as_str(), "open" | "completing")
            || (session.state == "open" && now()?.get() >= session.expires_at_ms)
        {
            return Err(not_found());
        }
        ProviderPartsListV1::new(
            reference.upload_id(),
            reference.key().clone(),
            self.parts(&session, context.correlation_id()).await?,
            context.correlation_id(),
        )
    }

    async fn put_part(
        &self,
        context: StorageRequestContext,
        request: ProviderPutPartRequestV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        self.require_provider_mutations_enabled().await?;
        let reference = request.reference();
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, reference, &session)?;
        let current = now()?;
        let geometry = self.intent_geometry(reference.upload_id()).await?;
        let request_size = u64::try_from(request.bytes().len()).map_err(|_| invalid())?;
        if session.state != "open"
            || current.get() >= session.expires_at_ms
            || request.bytes().is_empty()
            || request_size > self.capabilities.max_part_size().get()
            || &ChecksumSha256::digest_bytes(request.bytes()) != request.checksum_sha256()
        {
            return Err(invalid());
        }
        Self::validate_part_geometry(&session, &geometry, request.part_number(), request_size)?;
        let existing = self.parts(&session, context.correlation_id()).await?;
        if let Some(part) = existing
            .iter()
            .find(|part| part.part_number() == request.part_number())
        {
            return if part.size().get()
                == u64::try_from(request.bytes().len()).map_err(|_| invalid())?
                && part.checksum_sha256() == request.checksum_sha256()
            {
                Ok(part.clone())
            } else {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            };
        }
        let claim_token = self.claim_part(&session, &request, current).await?;
        let upload = self
            .bucket
            .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
            .map_err(map_worker_error)?;
        let uploaded = upload
            .upload_part(request.part_number().get(), request.bytes().to_vec())
            .into_send()
            .await;
        let uploaded = match uploaded {
            Ok(uploaded) => uploaded,
            Err(error) => {
                // Release only this invocation's lease. If D1 is unavailable,
                // retaining the bounded lease is safer than admitting a
                // concurrent different provider write.
                if let Ok(statement) = self
                    .database
                    .prepare(
                        "DELETE FROM r2_multipart_part_claims_v1 \
                         WHERE upload_id=?1 AND part_number=?2 AND claim_token=?3",
                    )
                    .bind(&[
                        JsValue::from_str(&reference.upload_id().to_string()),
                        JsValue::from_f64(f64::from(request.part_number().get())),
                        JsValue::from_str(&claim_token),
                    ])
                {
                    let _ = statement.run().into_send().await;
                }
                return Err(map_worker_error(error));
            }
        };
        let receipt = ProviderPartReceiptV1::new(
            reference.upload_id(),
            reference.key().clone(),
            request.part_number(),
            ByteSize::new(u64::try_from(request.bytes().len()).map_err(|_| invalid())?)
                .map_err(|_| invalid())?,
            request.checksum_sha256().clone(),
            ProviderEntityTag::parse(uploaded.etag())?,
            context.correlation_id(),
        );
        self.database
            .prepare(
                "INSERT INTO r2_multipart_parts_v1(upload_id,part_number,bytes,checksum_sha256,provider_etag,uploaded_at_ms,part_claim_token) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(upload_id,part_number) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&reference.upload_id().to_string()),
                JsValue::from_f64(f64::from(request.part_number().get())),
                number(receipt.size().get())?,
                JsValue::from_str(receipt.checksum_sha256().as_str()),
                JsValue::from_str(receipt.etag().expose_for_provider_comparison()),
                JsValue::from_f64(now()?.get() as f64),
                JsValue::from_str(&claim_token),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let stored = self
            .parts(&session, context.correlation_id())
            .await?
            .into_iter()
            .find(|part| part.part_number() == request.part_number())
            .ok_or_else(unavailable)?;
        if stored == receipt {
            Ok(stored)
        } else {
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        }
    }

    async fn complete_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCompleteMultipartRequestV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        self.require_provider_mutations_enabled().await?;
        let reference = request.reference();
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, reference, &session)?;
        if let Some(completed) = self.completion(&request, context.correlation_id()).await? {
            return Ok(completed);
        }
        let current = now()?;
        if !completion_state_admissible(&session.state, session.expires_at_ms, current.get()) {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        let stored_parts = self.parts(&session, context.correlation_id()).await?;
        let geometry = self.intent_geometry(reference.upload_id()).await?;
        Self::validate_completion_geometry(&session, &geometry, &stored_parts)?;
        if stored_parts != request.parts()
            || signed(request.expected_size().get())? != session.expected_bytes
            || request.expected_checksum_sha256().as_str() != session.checksum_sha256
            || request.expected_content_type().as_str() != session.content_type
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        let parts_digest = completion_parts_digest(request.parts());
        let completion_claim_token = self
            .claim_completion(reference.upload_id(), &parts_digest, current)
            .await?;
        if self
            .bucket
            .head(reference.key().as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_none()
        {
            let upload = self
                .bucket
                .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
                .map_err(map_worker_error)?;
            let parts = stored_parts
                .iter()
                .map(|part| {
                    UploadedPart::new(
                        part.part_number().get(),
                        part.etag().expose_for_provider_comparison().into(),
                    )
                })
                .collect::<Vec<_>>();
            let provider_completion = upload.complete(parts).into_send().await;
            if let Err(error) = provider_completion
                && self
                    .bucket
                    .head(reference.key().as_str())
                    .into_send()
                    .await
                    .map_err(map_worker_error)?
                    .is_none()
            {
                return Err(map_worker_error(error));
            }
        }
        let object = self
            .verify_full_object(
                reference.key(),
                request.expected_size(),
                request.expected_checksum_sha256(),
                request.expected_content_type(),
            )
            .await?;
        self.persist_verified_object(&request, &object, now()?)
            .await?;
        let probe = self
            .probe
            .probe(
                self.bucket,
                reference.key(),
                request.expected_content_type(),
                request.expected_size(),
                request.expected_checksum_sha256(),
            )
            .await?;
        let completed_at =
            timestamp(i64::try_from(object.uploaded().as_millis()).map_err(|_| integrity())?)?;
        if let Err(error) = self
            .persist_completion(
                &request,
                &object,
                &probe,
                &completion_claim_token,
                completed_at,
            )
            .await
        {
            return self
                .completion(&request, context.correlation_id())
                .await?
                .ok_or(error);
        }
        self.completion(&request, context.correlation_id())
            .await?
            .ok_or_else(unavailable)
    }

    async fn abort_multipart(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        self.require_provider_mutations_enabled().await?;
        let upload_id = reference.upload_id();
        let key = reference.key().clone();
        let correlation_id = reference.correlation_id();
        let now = now()?;
        let session = self.session(upload_id).await?.ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, &session)?;
        let attempt = match session.state.as_str() {
            "open" => self
                .begin_abort_attempt(&session, AbortIntentKind::AuthenticatedDelete, now)
                .await?
                .ok_or_else(unavailable)?,
            "completing" | "complete" => {
                return Ok(ProviderAbortReceiptV1::new(
                    upload_id,
                    key,
                    ProviderAbortDispositionV1::AlreadyCompleted,
                    correlation_id,
                ));
            }
            "aborted" | "expired" => {
                return Ok(ProviderAbortReceiptV1::new(
                    upload_id,
                    key,
                    ProviderAbortDispositionV1::AlreadyAborted,
                    correlation_id,
                ));
            }
            _ => return Err(integrity()),
        };
        let disposition = match self
            .reconcile_authenticated_abort_provider(context, reference, attempt, now)
            .await
        {
            Ok(AuthenticatedAbortOutcomeV1::Confirmed { attempt }) => {
                self.finish_abort_reconciliation(
                    &upload_id.to_string(),
                    AbortIntentKind::AuthenticatedDelete,
                    attempt,
                    AbortTerminalOutcome::Confirmed,
                    now,
                )
                .await?;
                ProviderAbortDispositionV1::Aborted
            }
            Ok(AuthenticatedAbortOutcomeV1::PreservedObject { attempt }) => {
                self.finish_abort_reconciliation(
                    &upload_id.to_string(),
                    AbortIntentKind::AuthenticatedDelete,
                    attempt,
                    AbortTerminalOutcome::PreservedObject,
                    now,
                )
                .await?;
                ProviderAbortDispositionV1::AlreadyCompleted
            }
            Ok(AuthenticatedAbortOutcomeV1::AlreadyAborted) => {
                ProviderAbortDispositionV1::AlreadyAborted
            }
            Ok(AuthenticatedAbortOutcomeV1::AlreadyCompleted) => {
                ProviderAbortDispositionV1::AlreadyCompleted
            }
            Ok(AuthenticatedAbortOutcomeV1::Pending) => return Err(unavailable()),
            Err(failure) => {
                self.retain_abort_failure(
                    &upload_id.to_string(),
                    AbortIntentKind::AuthenticatedDelete,
                    attempt,
                    &failure,
                    now,
                )
                .await?;
                return Err(failure);
            }
        };
        Ok(ProviderAbortReceiptV1::new(
            upload_id,
            key,
            disposition,
            correlation_id,
        ))
    }

    async fn head_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.range().is_some() || request.correlation_id() != context.correlation_id() {
            return Err(invalid());
        }
        let completed = self
            .completion_by_key(request.key(), context.correlation_id())
            .await?
            .ok_or_else(not_found)?;
        let metadata = Self::download_metadata(&completed, context.correlation_id());
        if let Some(response) = Self::condition(request.validator(), metadata.clone())? {
            return Ok(response);
        }
        Ok(ProviderDownloadResponseV1::Head(metadata))
    }

    async fn get_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.correlation_id() != context.correlation_id() {
            return Err(invalid());
        }
        let completed = self
            .completion_by_key(request.key(), context.correlation_id())
            .await?
            .ok_or_else(not_found)?;
        let metadata = Self::download_metadata(&completed, context.correlation_id());
        if let Some(response) = Self::condition(request.validator(), metadata.clone())? {
            return Ok(response);
        }
        let range = request.range().unwrap_or(frame_ports::ObjectByteRange::new(
            0,
            completed.size().get(),
        )?);
        if range.end_exclusive() > completed.size().get()
            || range.length() > self.capabilities.max_range_size().get()
        {
            return Err(invalid());
        }
        let object = self
            .bucket
            .get(request.key().as_str())
            .range(worker::Range::OffsetWithLength {
                offset: range.start(),
                length: range.length(),
            })
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(not_found)?;
        let stream = object
            .body()
            .ok_or_else(integrity)?
            .stream()
            .map_err(map_worker_error)?;
        Ok(ProviderDownloadResponseV1::Body {
            metadata,
            range,
            body: Box::new(R2DownloadBodyV1(SendWrapper::new(stream))),
        })
    }
}

impl<P: TrustedR2MediaProbeV1 + ?Sized> R2MultipartObjectStoreV1<'_, P> {
    async fn begin_completion_reconciliation_attempt(
        &self,
        upload_id: &str,
        current: TimestampMillis,
    ) -> Result<Option<i64>, StorageFailure> {
        let lock_until = current
            .get()
            .checked_add(COMPLETION_CLAIM_LEASE_MS)
            .filter(|value| {
                *value <= i64::try_from(frame_domain::MAX_WIRE_INTEGER).unwrap_or(i64::MAX)
            })
            .ok_or_else(integrity)?;
        let acquired = self
            .database
            .prepare(
                "UPDATE r2_multipart_completion_reconciliation_v1 \
                 SET attempt_count=attempt_count+1,next_attempt_at_ms=?2,\
                   last_failure_class=NULL,updated_at_ms=?3 \
                 WHERE upload_id=?1 AND state='pending' AND next_attempt_at_ms<=?3 \
                   AND attempt_count<?4 RETURNING attempt_count",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(lock_until as f64),
                JsValue::from_f64(current.get() as f64),
                JsValue::from_f64(MAX_COMPLETION_RECONCILIATION_ATTEMPTS as f64),
            ])
            .map_err(map_worker_error)?
            .first::<CompletionReconciliationAttemptRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        Ok(acquired.map(|row| row.attempt_count))
    }

    async fn retain_completion_reconciliation_failure(
        &self,
        upload_id: &str,
        attempt: i64,
        failure: &StorageFailure,
        current: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let class = abort_failure_class(failure.kind());
        let quarantined = completion_failure_is_terminal(failure.kind(), attempt);
        let next_attempt_at_ms = if quarantined {
            current.get()
        } else {
            completion_retry_at(current.get(), attempt, failure.retry_after())
        };
        let statement = if quarantined {
            self.database
                .prepare(
                    "UPDATE r2_multipart_completion_reconciliation_v1 \
                     SET state='quarantined',next_attempt_at_ms=?3,last_failure_class=?4,\
                       updated_at_ms=?3,terminal_at_ms=?3 \
                     WHERE upload_id=?1 AND state='pending' AND attempt_count=?2 \
                       AND last_failure_class IS NULL",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_f64(attempt as f64),
                    JsValue::from_f64(next_attempt_at_ms as f64),
                    JsValue::from_str(class),
                ])
        } else {
            self.database
                .prepare(
                    "UPDATE r2_multipart_completion_reconciliation_v1 \
                     SET next_attempt_at_ms=?3,last_failure_class=?4,updated_at_ms=?5 \
                     WHERE upload_id=?1 AND state='pending' AND attempt_count=?2 \
                       AND last_failure_class IS NULL",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_f64(attempt as f64),
                    JsValue::from_f64(next_attempt_at_ms as f64),
                    JsValue::from_str(class),
                    JsValue::from_f64(current.get() as f64),
                ])
        }
        .map_err(map_worker_error)?;
        let result = statement
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !result.success() {
            return Err(unavailable());
        }
        let stored = self
            .database
            .prepare(
                "SELECT state,attempt_count,next_attempt_at_ms,last_failure_class,terminal_at_ms \
                 FROM r2_multipart_completion_reconciliation_v1 \
                 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(upload_id)])
            .map_err(map_worker_error)?
            .first::<CompletionReconciliationRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let expected_state = if quarantined {
            "quarantined"
        } else {
            "pending"
        };
        if stored.is_some_and(|row| {
            row.state == expected_state
                && row.attempt_count == attempt
                && row.next_attempt_at_ms == next_attempt_at_ms
                && row.last_failure_class.as_deref() == Some(class)
                && row.terminal_at_ms == quarantined.then_some(current.get())
        }) {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    async fn quarantine_exhausted_completion_reconciliation(
        &self,
        upload_id: &str,
        current: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let result = self
            .database
            .prepare(
                "UPDATE r2_multipart_completion_reconciliation_v1 \
                 SET state='quarantined',next_attempt_at_ms=?2,\
                   last_failure_class='unavailable',updated_at_ms=?2,terminal_at_ms=?2 \
                 WHERE upload_id=?1 AND state='pending' AND attempt_count=?3 \
                   AND next_attempt_at_ms<=?2",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(current.get() as f64),
                JsValue::from_f64(MAX_COMPLETION_RECONCILIATION_ATTEMPTS as f64),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if !result.success() {
            return Err(unavailable());
        }
        let stored = self
            .database
            .prepare(
                "SELECT state,attempt_count,next_attempt_at_ms,last_failure_class,terminal_at_ms \
                 FROM r2_multipart_completion_reconciliation_v1 \
                 WHERE upload_id=?1 LIMIT 1",
            )
            .bind(&[JsValue::from_str(upload_id)])
            .map_err(map_worker_error)?
            .first::<CompletionReconciliationRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if stored.is_some_and(|row| {
            row.state == "quarantined"
                && row.attempt_count == MAX_COMPLETION_RECONCILIATION_ATTEMPTS
                && row.next_attempt_at_ms == current.get()
                && row.last_failure_class.as_deref() == Some("unavailable")
                && row.terminal_at_ms == Some(current.get())
        }) {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    async fn replay_completing_upload(&self, upload_id: &str) -> Result<(), StorageFailure> {
        let upload_id = MultipartUploadId::parse(upload_id).map_err(|_| integrity())?;
        let row = self.session(upload_id).await?.ok_or_else(integrity)?;
        let key = row.key()?;
        let correlation = correlation(&row.correlation_id)?;
        let context = StorageRequestContext::new(key.tenant_id(), correlation);
        let reference = ProviderUploadReferenceV1::new(
            row.upload()?,
            key,
            ProviderMultipartHandleV1::parse(encode_handle(&row.provider_upload_id))?,
            correlation,
        );
        let parts = self.parts(&row, correlation).await?;
        let request = ProviderCompleteMultipartRequestV1::new(
            reference,
            parts,
            ByteSize::new(u64::try_from(row.expected_bytes).map_err(|_| integrity())?)
                .map_err(|_| integrity())?,
            ChecksumSha256::parse(row.checksum_sha256).map_err(|_| integrity())?,
            ContentType::parse(row.content_type).map_err(|_| integrity())?,
        )?;
        self.complete_multipart(context, request).await?;
        Ok(())
    }

    /// Replays one provider-completed/D1-pending completion. This is invoked
    /// by the Worker scheduler so a lost HTTP acknowledgement does not require
    /// the desktop to remain online while a trusted probe finishes. Candidate
    /// authority lives in the reconciliation journal: a permanent failure is
    /// quarantined, and a retryable failure is leased/backed off, so neither
    /// can keep a later completing session permanently hidden.
    pub async fn reconcile_completing_one(&self) -> Result<bool, StorageFailure> {
        if !self.provider_mutations_enabled().await? {
            return Ok(false);
        }
        let current = now()?;
        let candidate = self
            .database
            .prepare(
                "SELECT reconciliation.upload_id,reconciliation.attempt_count \
                 FROM r2_multipart_completion_reconciliation_v1 reconciliation \
                 JOIN r2_multipart_sessions_v1 session USING(upload_id) \
                 WHERE reconciliation.state='pending' \
                   AND reconciliation.next_attempt_at_ms<=?1 \
                   AND session.state='completing' \
                 ORDER BY reconciliation.next_attempt_at_ms,session.created_at_ms,\
                   reconciliation.upload_id LIMIT 1",
            )
            .bind(&[JsValue::from_f64(current.get() as f64)])
            .map_err(map_worker_error)?
            .first::<CompletionReconciliationCandidateRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let Some(candidate) = candidate else {
            return Ok(false);
        };
        if candidate.attempt_count >= MAX_COMPLETION_RECONCILIATION_ATTEMPTS {
            self.quarantine_exhausted_completion_reconciliation(&candidate.upload_id, current)
                .await?;
            return Ok(true);
        }
        let Some(attempt) = self
            .begin_completion_reconciliation_attempt(&candidate.upload_id, current)
            .await?
        else {
            return Ok(false);
        };
        match self.replay_completing_upload(&candidate.upload_id).await {
            Ok(()) => Ok(true),
            Err(failure) => {
                let failed_at = now()?;
                self.retain_completion_reconciliation_failure(
                    &candidate.upload_id,
                    attempt,
                    &failure,
                    failed_at,
                )
                .await?;
                Err(failure)
            }
        }
    }

    async fn completion_by_key(
        &self,
        key: &ScopedObjectKey,
        correlation_id: CorrelationId,
    ) -> Result<Option<ProviderCompletedObjectV1>, StorageFailure> {
        let row = self
            .database
            .prepare(
                "SELECT s.upload_id,c.request_parts_sha256,c.provider_version,c.provider_etag,c.bytes,c.checksum_sha256,c.content_type,\
                 c.container,c.video_codec,c.audio_codec,c.width,c.height,c.duration_ms,\
                 c.frame_rate_millihertz,c.completed_at_ms,c.correlation_id \
                 FROM r2_multipart_sessions_v1 s JOIN r2_multipart_completions_v1 c USING(upload_id) \
                 WHERE s.object_key=?1 AND s.state='complete' LIMIT 1",
            )
            .bind(&[JsValue::from_str(key.as_str())])
            .map_err(map_worker_error)?
            .first::<CompletionWithIdRow>(None)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        row.map(|row| {
            let upload = MultipartUploadId::parse(&row.upload_id).map_err(|_| integrity())?;
            row.completion
                .completed(upload, key.clone(), correlation_id)
        })
        .transpose()
    }
}

struct R2DownloadBodyV1(SendWrapper<worker::ByteStream>);

#[async_trait]
impl ProviderDownloadBodyV1 for R2DownloadBodyV1 {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure> {
        self.0
            .try_next()
            .into_send()
            .await
            .map_err(map_worker_error)
    }
}

#[derive(Debug, Deserialize)]
struct PresenceRow {
    present: i64,
}

#[derive(Debug, Deserialize)]
struct MultipartIntentGeometryRow {
    part_size: i64,
    part_count: i64,
}

#[derive(Deserialize)]
struct CreationClaimRow {
    upload_id: String,
    organization_id: String,
    object_key: String,
    expected_bytes: i64,
    checksum_sha256: String,
    content_type: String,
    correlation_id: String,
    part_size: i64,
    part_count: i64,
    expires_at_ms: i64,
    claim_token: String,
    state: String,
    provider_upload_id: Option<String>,
    created_at_ms: i64,
}

struct CreationReservation {
    claim: CreationClaimRow,
    acquired: bool,
}

#[derive(Deserialize)]
struct PartClaimTokenRow {
    claim_token: String,
}

#[derive(Debug, Deserialize)]
struct PartClaimShapeRow {
    bytes: i64,
    checksum_sha256: String,
}

#[derive(Debug, Deserialize)]
struct CompletionClaimShapeRow {
    request_parts_sha256: String,
    claim_token: String,
    state: String,
    attempt_count: i64,
    lease_expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct CompletionReconciliationCandidateRow {
    upload_id: String,
    attempt_count: i64,
}

#[derive(Debug, Deserialize)]
struct CompletionReconciliationAttemptRow {
    attempt_count: i64,
}

#[derive(Debug, Deserialize)]
struct CompletionReconciliationRow {
    state: String,
    attempt_count: i64,
    next_attempt_at_ms: i64,
    last_failure_class: Option<String>,
    terminal_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TrustedProbeRow {
    container: String,
    video_codec: String,
    audio_codec: String,
    width: i64,
    height: i64,
    duration_ms: i64,
    frame_rate_numerator: i64,
    frame_rate_denominator: i64,
}

#[derive(Debug, Deserialize)]
struct VerifiedObjectRow {
    provider_version: String,
    provider_etag: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
}

fn abort_change_assertion(
    database: &D1Database,
    operation_id: &str,
    upload_id: &str,
    assertion_kind: &str,
) -> Result<worker::D1PreparedStatement, StorageFailure> {
    database
        .prepare(
            "INSERT INTO r2_multipart_abort_batch_assertions_v1(\
             operation_id,upload_id,assertion_kind,expected_count,actual_count) \
             VALUES(?1,?2,?3,1,changes())",
        )
        .bind(&[
            JsValue::from_str(operation_id),
            JsValue::from_str(upload_id),
            JsValue::from_str(assertion_kind),
        ])
        .map_err(map_worker_error)
}

#[derive(Debug, Deserialize)]
struct AbortReconciliationRow {
    intent_kind: String,
    state: String,
    attempt_count: i64,
    next_attempt_at_ms: i64,
    last_failure_class: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortTerminalOutcome {
    Confirmed,
    PreservedObject,
}

impl AbortTerminalOutcome {
    const fn session_state(self, intent: AbortIntentKind) -> &'static str {
        match (self, intent) {
            (Self::Confirmed, AbortIntentKind::ExpiryCleanup) => "expired",
            (Self::Confirmed, AbortIntentKind::AuthenticatedDelete) => "aborted",
            (Self::PreservedObject, _) => "completing",
        }
    }

    const fn reconciliation_state(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::PreservedObject => "preserved_object",
        }
    }
}

#[derive(Debug, Deserialize)]
struct SessionRow {
    upload_id: String,
    object_key: String,
    provider_upload_id: String,
    state: String,
    expected_bytes: i64,
    checksum_sha256: String,
    content_type: String,
    correlation_id: String,
    #[serde(rename = "created_at_ms")]
    _created_at_ms: i64,
    expires_at_ms: i64,
    #[serde(rename = "completed_at_ms")]
    _completed_at_ms: Option<i64>,
}

impl SessionRow {
    fn upload(&self) -> Result<MultipartUploadId, StorageFailure> {
        MultipartUploadId::parse(&self.upload_id).map_err(|_| integrity())
    }

    fn key(&self) -> Result<ScopedObjectKey, StorageFailure> {
        ScopedObjectKey::parse(&self.object_key).map_err(|_| integrity())
    }
}

#[derive(Debug, Deserialize)]
struct PartRow {
    part_number: i64,
    bytes: i64,
    checksum_sha256: String,
    provider_etag: String,
    uploaded_at_ms: i64,
}

impl PartRow {
    fn receipt(
        self,
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        correlation: CorrelationId,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        timestamp(self.uploaded_at_ms)?;
        Ok(ProviderPartReceiptV1::new(
            upload_id,
            key,
            MultipartPartNumberV1::new(u16::try_from(self.part_number).map_err(|_| integrity())?)
                .map_err(|_| integrity())?,
            ByteSize::new(u64::try_from(self.bytes).map_err(|_| integrity())?)
                .map_err(|_| integrity())?,
            ChecksumSha256::parse(self.checksum_sha256).map_err(|_| integrity())?,
            ProviderEntityTag::parse(self.provider_etag).map_err(|_| integrity())?,
            correlation,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct CompletionRow {
    request_parts_sha256: String,
    provider_version: String,
    provider_etag: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
    container: String,
    video_codec: String,
    audio_codec: String,
    width: i64,
    height: i64,
    duration_ms: i64,
    frame_rate_millihertz: i64,
    completed_at_ms: i64,
    correlation_id: String,
}

fn completion_parts_digest(parts: &[ProviderPartReceiptV1]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame/r2/multipart-completion-parts/v1\0");
    for part in parts {
        digest.update(part.part_number().get().to_be_bytes());
        digest.update(part.size().get().to_be_bytes());
        let checksum = part.checksum_sha256().as_str().as_bytes();
        digest.update((checksum.len() as u64).to_be_bytes());
        digest.update(checksum);
        let etag = part.etag().expose_for_provider_comparison().as_bytes();
        digest.update((etag.len() as u64).to_be_bytes());
        digest.update(etag);
    }
    hex(&digest.finalize())
}

fn completion_state_admissible(state: &str, expires_at_ms: i64, current_ms: i64) -> bool {
    match state {
        "open" => current_ms < expires_at_ms,
        // A durable completion claim linearized before expiry. Only the same
        // ordered-parts digest may take over its expired execution lease.
        "completing" => true,
        _ => false,
    }
}

impl CompletionRow {
    fn completed(
        self,
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        correlation_id: CorrelationId,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        correlation(&self.correlation_id)?;
        let probe = TrustedMediaProbeV1::new(
            parse_container(&self.container)?,
            parse_video_codec(&self.video_codec)?,
            parse_audio_codec(&self.audio_codec)?,
            u16::try_from(self.width).map_err(|_| integrity())?,
            u16::try_from(self.height).map_err(|_| integrity())?,
            u64::try_from(self.duration_ms).map_err(|_| integrity())?,
            u32::try_from(self.frame_rate_millihertz).map_err(|_| integrity())?,
        )
        .map_err(|_| integrity())?;
        Ok(ProviderCompletedObjectV1::new(
            upload_id,
            key,
            ByteSize::new(u64::try_from(self.bytes).map_err(|_| integrity())?)
                .map_err(|_| integrity())?,
            ChecksumSha256::parse(self.checksum_sha256).map_err(|_| integrity())?,
            ContentType::parse(self.content_type).map_err(|_| integrity())?,
            ProviderObjectVersion::parse(self.provider_version).map_err(|_| integrity())?,
            ProviderEntityTag::parse(self.provider_etag).map_err(|_| integrity())?,
            timestamp(self.completed_at_ms)?,
            probe,
            correlation_id,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct CompletionWithIdRow {
    upload_id: String,
    #[serde(flatten)]
    completion: CompletionRow,
}

fn now() -> Result<TimestampMillis, StorageFailure> {
    timestamp(js_sys::Date::now().round() as i64)
}

fn timestamp(value: i64) -> Result<TimestampMillis, StorageFailure> {
    TimestampMillis::new(value).map_err(|_| integrity())
}

fn correlation(value: &str) -> Result<CorrelationId, StorageFailure> {
    CorrelationId::parse(value).map_err(|_| integrity())
}

fn signed(value: u64) -> Result<i64, StorageFailure> {
    i64::try_from(value).map_err(|_| invalid())
}

fn number(value: u64) -> Result<JsValue, StorageFailure> {
    if value > frame_domain::MAX_WIRE_INTEGER {
        return Err(invalid());
    }
    Ok(JsValue::from_f64(value as f64))
}

fn encode_handle(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame/r2-multipart-handle/v1\0");
    digest.update(value.as_bytes());
    hex(&digest.finalize())
}

fn hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        output.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    output
}

const fn container_name(value: MediaContainerV1) -> &'static str {
    match value {
        MediaContainerV1::Webm => "webm",
        MediaContainerV1::Mp4 => "mp4",
        MediaContainerV1::QuickTime => "quicktime",
        MediaContainerV1::Matroska => "matroska",
    }
}

const fn video_codec_name(value: VideoCodecV1) -> &'static str {
    match value {
        VideoCodecV1::H264 => "h264",
        VideoCodecV1::H265 => "h265",
        VideoCodecV1::Vp8 => "vp8",
        VideoCodecV1::Vp9 => "vp9",
        VideoCodecV1::Av1 => "av1",
    }
}

const fn audio_codec_name(value: AudioCodecV1) -> &'static str {
    match value {
        AudioCodecV1::None => "none",
        AudioCodecV1::Aac => "aac",
        AudioCodecV1::Opus => "opus",
    }
}

fn parse_container(value: &str) -> Result<MediaContainerV1, StorageFailure> {
    match value {
        "webm" => Ok(MediaContainerV1::Webm),
        "mp4" => Ok(MediaContainerV1::Mp4),
        "quicktime" => Ok(MediaContainerV1::QuickTime),
        "matroska" => Ok(MediaContainerV1::Matroska),
        _ => Err(integrity()),
    }
}

fn parse_video_codec(value: &str) -> Result<VideoCodecV1, StorageFailure> {
    match value {
        "h264" => Ok(VideoCodecV1::H264),
        "h265" => Ok(VideoCodecV1::H265),
        "vp8" => Ok(VideoCodecV1::Vp8),
        "vp9" => Ok(VideoCodecV1::Vp9),
        "av1" => Ok(VideoCodecV1::Av1),
        _ => Err(integrity()),
    }
}

fn parse_audio_codec(value: &str) -> Result<AudioCodecV1, StorageFailure> {
    match value {
        "none" => Ok(AudioCodecV1::None),
        "aac" => Ok(AudioCodecV1::Aac),
        "opus" => Ok(AudioCodecV1::Opus),
        _ => Err(integrity()),
    }
}

fn map_worker_error(error: worker::Error) -> StorageFailure {
    let message = error.to_string().to_ascii_lowercase();
    let kind = if message.contains("404") || message.contains("not found") {
        StorageFailureKind::NotFound
    } else if message.contains("412") || message.contains("precondition") {
        StorageFailureKind::PreconditionFailed
    } else if message.contains("401") || message.contains("403") || message.contains("forbidden") {
        StorageFailureKind::Unauthorized
    } else if message.contains("429") || message.contains("rate") {
        StorageFailureKind::Throttled
    } else if message.contains("timeout") {
        StorageFailureKind::Timeout
    } else {
        StorageFailureKind::Unavailable
    };
    StorageFailure::new(kind)
}

pub const fn abort_failure_class(kind: StorageFailureKind) -> &'static str {
    match kind {
        StorageFailureKind::NotFound => "not_found",
        StorageFailureKind::PreconditionFailed => "precondition_failed",
        StorageFailureKind::Throttled => "throttled",
        StorageFailureKind::Unauthorized => "unauthorized",
        StorageFailureKind::QuotaExceeded => "quota_exceeded",
        StorageFailureKind::Timeout => "timeout",
        StorageFailureKind::Integrity => "integrity",
        StorageFailureKind::Unavailable => "unavailable",
        StorageFailureKind::UnsupportedCapability => "unsupported_capability",
        StorageFailureKind::InvalidRequest => "invalid_request",
    }
}

pub fn abort_retry_at(now_ms: i64, attempt: i64) -> i64 {
    let shift = u32::try_from(attempt.saturating_sub(1).min(18)).unwrap_or(18);
    let delay = ABORT_RETRY_BASE_MS
        .saturating_mul(1_i64.checked_shl(shift).unwrap_or(i64::MAX))
        .min(ABORT_RETRY_MAX_MS);
    now_ms.saturating_add(delay)
}

pub fn completion_retry_at(now_ms: i64, attempt: i64, retry_after: Option<DurationMillis>) -> i64 {
    let shift = u32::try_from(attempt.saturating_sub(1).min(18)).unwrap_or(18);
    let exponential_delay =
        COMPLETION_RETRY_BASE_MS.saturating_mul(1_i64.checked_shl(shift).unwrap_or(i64::MAX));
    let provider_delay = retry_after
        .and_then(|delay| i64::try_from(delay.get()).ok())
        .unwrap_or(0);
    let delay = exponential_delay
        .max(COMPLETION_CLAIM_LEASE_MS)
        .max(provider_delay)
        .min(COMPLETION_RETRY_MAX_MS);
    now_ms.saturating_add(delay)
}

pub const fn completion_failure_is_terminal(kind: StorageFailureKind, attempt: i64) -> bool {
    !matches!(
        kind,
        StorageFailureKind::Throttled
            | StorageFailureKind::Timeout
            | StorageFailureKind::Unavailable
    ) || attempt >= MAX_COMPLETION_RECONCILIATION_ATTEMPTS
}

pub const fn abort_attempt_lock_until(now_ms: i64) -> i64 {
    now_ms.saturating_add(ABORT_ATTEMPT_LOCK_MS)
}

const fn invalid() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::InvalidRequest)
}

const fn integrity() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Integrity)
}

const fn not_found() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::NotFound)
}

const fn unavailable() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry_session(expected_bytes: i64) -> SessionRow {
        SessionRow {
            upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into(),
            object_key: "unused-in-geometry-test".into(),
            provider_upload_id: "unused-provider-handle".into(),
            state: "open".into(),
            expected_bytes,
            checksum_sha256: "01".repeat(32),
            content_type: "video/webm".into(),
            correlation_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into(),
            _created_at_ms: 1,
            expires_at_ms: 10,
            _completed_at_ms: None,
        }
    }

    #[test]
    fn provider_handles_are_opaque_safe_and_deterministic() {
        let raw = "r2/provider+upload=id";
        let encoded = encode_handle(raw);
        assert_eq!(encoded, encode_handle(raw));
        assert_ne!(encoded, encode_handle("r2/provider+upload=other"));
        assert_eq!(encoded.len(), 64);
        assert!(!encoded.contains(raw));
        assert!(ProviderMultipartHandleV1::parse(encoded).is_ok());
    }

    #[test]
    fn persisted_probe_enums_cover_every_domain_variant() {
        for value in [
            MediaContainerV1::Webm,
            MediaContainerV1::Mp4,
            MediaContainerV1::QuickTime,
            MediaContainerV1::Matroska,
        ] {
            assert_eq!(parse_container(container_name(value)), Ok(value));
        }
        for value in [
            VideoCodecV1::H264,
            VideoCodecV1::H265,
            VideoCodecV1::Vp8,
            VideoCodecV1::Vp9,
            VideoCodecV1::Av1,
        ] {
            assert_eq!(parse_video_codec(video_codec_name(value)), Ok(value));
        }
        for value in [AudioCodecV1::None, AudioCodecV1::Aac, AudioCodecV1::Opus] {
            assert_eq!(parse_audio_codec(audio_codec_name(value)), Ok(value));
        }
    }

    #[test]
    fn abort_failure_classes_are_exhaustive_and_backoff_is_bounded() {
        for (kind, class) in [
            (StorageFailureKind::NotFound, "not_found"),
            (
                StorageFailureKind::PreconditionFailed,
                "precondition_failed",
            ),
            (StorageFailureKind::Throttled, "throttled"),
            (StorageFailureKind::Unauthorized, "unauthorized"),
            (StorageFailureKind::QuotaExceeded, "quota_exceeded"),
            (StorageFailureKind::Timeout, "timeout"),
            (StorageFailureKind::Integrity, "integrity"),
            (StorageFailureKind::Unavailable, "unavailable"),
            (
                StorageFailureKind::UnsupportedCapability,
                "unsupported_capability",
            ),
            (StorageFailureKind::InvalidRequest, "invalid_request"),
        ] {
            assert_eq!(abort_failure_class(kind), class);
        }
        let mut previous = 0;
        for attempt in 1..=32 {
            let next = abort_retry_at(0, attempt);
            assert!(next >= previous);
            assert!(next <= ABORT_RETRY_MAX_MS);
            previous = next;
        }
        assert_eq!(abort_retry_at(i64::MAX - 1, 32), i64::MAX);
    }

    #[test]
    fn completion_reconciliation_quarantines_permanent_and_exhausted_failures() {
        for kind in [
            StorageFailureKind::NotFound,
            StorageFailureKind::PreconditionFailed,
            StorageFailureKind::Unauthorized,
            StorageFailureKind::QuotaExceeded,
            StorageFailureKind::Integrity,
            StorageFailureKind::UnsupportedCapability,
            StorageFailureKind::InvalidRequest,
        ] {
            assert!(completion_failure_is_terminal(kind, 1));
        }
        for kind in [
            StorageFailureKind::Throttled,
            StorageFailureKind::Timeout,
            StorageFailureKind::Unavailable,
        ] {
            assert!(!completion_failure_is_terminal(kind, 1));
            assert!(completion_failure_is_terminal(
                kind,
                MAX_COMPLETION_RECONCILIATION_ATTEMPTS
            ));
        }
        let mut previous = 0;
        for attempt in 1..=MAX_COMPLETION_RECONCILIATION_ATTEMPTS {
            let next = completion_retry_at(0, attempt, None);
            assert!(next >= COMPLETION_CLAIM_LEASE_MS);
            assert!(next >= previous);
            assert!(next <= COMPLETION_RETRY_MAX_MS);
            previous = next;
        }
        let oversized_provider_delay = DurationMillis::new(
            u64::try_from(COMPLETION_RETRY_MAX_MS * 2).expect("positive retry delay"),
        )
        .expect("bounded duration");
        assert_eq!(
            completion_retry_at(0, 1, Some(oversized_provider_delay)),
            COMPLETION_RETRY_MAX_MS
        );
        assert_eq!(completion_retry_at(i64::MAX - 1, 32, None), i64::MAX);
    }

    #[test]
    fn abort_terminal_outcomes_never_conflate_expiry_with_preservation() {
        assert_eq!(
            AbortTerminalOutcome::Confirmed.session_state(AbortIntentKind::ExpiryCleanup),
            "expired"
        );
        assert_eq!(
            AbortTerminalOutcome::Confirmed.session_state(AbortIntentKind::AuthenticatedDelete),
            "aborted"
        );
        assert_eq!(
            AbortTerminalOutcome::Confirmed.reconciliation_state(),
            "confirmed"
        );
        assert_eq!(
            AbortTerminalOutcome::PreservedObject.session_state(AbortIntentKind::ExpiryCleanup),
            "completing"
        );
        assert_eq!(
            AbortTerminalOutcome::PreservedObject.reconciliation_state(),
            "preserved_object"
        );
    }

    #[test]
    fn completion_must_linearize_before_expiry_but_may_reconcile_afterward() {
        assert!(completion_state_admissible("open", 10, 9));
        assert!(!completion_state_admissible("open", 10, 10));
        assert!(!completion_state_admissible("open", 10, 11));
        assert!(completion_state_admissible("completing", 10, 10));
        assert!(completion_state_admissible("completing", 10, 11));
        for terminal in ["complete", "aborted", "expired", "unknown"] {
            assert!(!completion_state_admissible(terminal, 10, 9));
        }
    }

    #[test]
    fn persisted_geometry_rejects_gaps_oversized_parts_and_wrong_tail() {
        let geometry = MultipartIntentGeometryRow {
            part_size: 5,
            part_count: 3,
        };
        let session = geometry_session(12);
        assert!(
            R2MultipartObjectStoreV1::<D1TrustedMediaProbeV1<'_>>::validate_part_geometry(
                &session,
                &geometry,
                MultipartPartNumberV1::new(1).expect("part"),
                5,
            )
            .is_ok()
        );
        assert!(
            R2MultipartObjectStoreV1::<D1TrustedMediaProbeV1<'_>>::validate_part_geometry(
                &session,
                &geometry,
                MultipartPartNumberV1::new(3).expect("part"),
                2,
            )
            .is_ok()
        );
        for (part, size) in [(1, 4), (2, 6), (3, 3), (4, 1)] {
            assert!(
                R2MultipartObjectStoreV1::<D1TrustedMediaProbeV1<'_>>::validate_part_geometry(
                    &session,
                    &geometry,
                    MultipartPartNumberV1::new(part).expect("part"),
                    size,
                )
                .is_err()
            );
        }
    }
}
