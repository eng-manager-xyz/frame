//! Restart-safe R2 multipart provider adapter.
//!
//! Provider upload IDs and part receipts are persisted in D1. Completion
//! streams the finished object through SHA-256 before committing its receipt;
//! a trusted probe is injected explicitly and cannot be supplied by clients.

use async_trait::async_trait;
use frame_domain::{
    AudioCodecV1, ByteSize, ChecksumSha256, ContentType, CorrelationId, MediaContainerV1,
    MultipartPartNumberV1, MultipartUploadId, ScopedObjectKey, TimestampMillis,
    TrustedMediaProbeV1, VideoCodecV1,
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

    async fn persist_completion(
        &self,
        request: &ProviderCompleteMultipartRequestV1,
        object: &worker::Object,
        probe: &TrustedMediaProbeV1,
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
                     completed_at_ms,correlation_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) \
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
                ])
                .map_err(map_worker_error)?,
            self.database
                .prepare(
                    "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=?2 \
                     WHERE upload_id=?1 AND state IN ('open','completing') \
                       AND EXISTS (SELECT 1 FROM r2_multipart_completions_v1 c \
                         WHERE c.upload_id=?1 AND c.request_parts_sha256=?3 AND c.bytes=?4 \
                           AND c.checksum_sha256=?5 AND c.content_type=?6 AND c.correlation_id=?7)",
                )
                .bind(&[
                    JsValue::from_str(&reference.upload_id().to_string()),
                    JsValue::from_f64(completed_at.get() as f64),
                    JsValue::from_str(&parts_digest),
                    number(request.expected_size().get())?,
                    JsValue::from_str(request.expected_checksum_sha256().as_str()),
                    JsValue::from_str(request.expected_content_type().as_str()),
                    JsValue::from_str(&reference.correlation_id().to_string()),
                ])
                .map_err(map_worker_error)?,
        ];
        self.database
            .batch(statements)
            .into_send()
            .await
            .map_err(map_worker_error)?;
        Ok(())
    }

    pub async fn cleanup_stale(
        &self,
        now: TimestampMillis,
        limit: u16,
    ) -> Result<u16, StorageFailure> {
        if !(1..=100).contains(&limit) {
            return Err(invalid());
        }
        let rows = self
            .database
            .prepare(
                "SELECT upload_id,object_key,provider_upload_id,state,expected_bytes,checksum_sha256,\
                 content_type,correlation_id,created_at_ms,expires_at_ms,completed_at_ms \
                 FROM r2_multipart_sessions_v1 WHERE state IN ('open','completing') \
                 AND expires_at_ms<=?1 ORDER BY expires_at_ms,upload_id LIMIT ?2",
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
            if let Ok(upload) = self
                .bucket
                .resume_multipart_upload(&row.object_key, &row.provider_upload_id)
            {
                let _ = upload.abort().into_send().await;
            }
            self.database
                .prepare(
                    "UPDATE r2_multipart_sessions_v1 SET state='expired' \
                     WHERE upload_id=?1 AND state IN ('open','completing') AND expires_at_ms<=?2",
                )
                .bind(&[
                    JsValue::from_str(&row.upload_id),
                    JsValue::from_f64(now.get() as f64),
                ])
                .map_err(map_worker_error)?
                .run()
                .into_send()
                .await
                .map_err(map_worker_error)?;
            cleaned = cleaned.saturating_add(1);
        }
        Ok(cleaned)
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
        if request.correlation_id() != context.correlation_id()
            || request.spec().total_size() > self.capabilities.max_total_size()
            || request.spec().part_size() < self.capabilities.min_part_size()
            || request.spec().part_size() > self.capabilities.max_part_size()
            || request.spec().part_count() > self.capabilities.max_part_count()
        {
            return Err(invalid());
        }
        if let Some(existing) = self.session(request.upload_id()).await? {
            let exact = existing.object_key == request.key().as_str()
                && existing.expected_bytes == signed(request.spec().total_size().get())?
                && existing.checksum_sha256 == request.spec().checksum_sha256().as_str()
                && existing.content_type == request.spec().content_type().as_str()
                && existing.expires_at_ms == request.expires_at().get()
                && existing.correlation_id == context.correlation_id().to_string();
            return if exact && matches!(existing.state.as_str(), "open" | "completing") {
                Self::provider_session(&existing)
            } else {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            };
        }
        let authorized = self
            .database
            .prepare(
                "SELECT 1 AS present FROM video_uploads WHERE id=?1 AND organization_id=?2 \
                 AND source_object_key=?3 AND expected_bytes=?4 AND content_type=?5 \
                 AND state IN ('initiated','uploading') LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&request.upload_id().to_string()),
                JsValue::from_str(&context.tenant_id().to_string()),
                JsValue::from_str(request.key().as_str()),
                number(request.spec().total_size().get())?,
                JsValue::from_str(request.spec().content_type().as_str()),
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
        let metadata = HttpMetadata {
            content_type: Some(request.spec().content_type().as_str().into()),
            content_disposition: Some("attachment".into()),
            cache_control: Some("private, no-store".into()),
            ..HttpMetadata::default()
        };
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
            ]))
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let provider_id = upload.upload_id().into_send().await;
        ProviderMultipartHandleV1::parse(encode_handle(&provider_id))?;
        let now = now()?;
        let inserted = self
            .database
            .prepare(
                "INSERT INTO r2_multipart_sessions_v1(\
                 upload_id,object_key,provider_upload_id,state,expected_bytes,checksum_sha256,\
                 content_type,correlation_id,created_at_ms,expires_at_ms,completed_at_ms) \
                 VALUES(?1,?2,?3,'open',?4,?5,?6,?7,?8,?9,NULL)",
            )
            .bind(&[
                JsValue::from_str(&request.upload_id().to_string()),
                JsValue::from_str(request.key().as_str()),
                JsValue::from_str(&provider_id),
                number(request.spec().total_size().get())?,
                JsValue::from_str(request.spec().checksum_sha256().as_str()),
                JsValue::from_str(request.spec().content_type().as_str()),
                JsValue::from_str(&context.correlation_id().to_string()),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_f64(request.expires_at().get() as f64),
            ])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await;
        if inserted.is_err() {
            let _ = upload.abort().into_send().await;
            let existing = self
                .session(request.upload_id())
                .await?
                .ok_or_else(unavailable)?;
            return Self::provider_session(&existing);
        }
        Self::provider_session(
            &self
                .session(request.upload_id())
                .await?
                .ok_or_else(unavailable)?,
        )
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
        if matches!(session.state.as_str(), "aborted" | "expired" | "complete") {
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
        let reference = request.reference();
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, reference, &session)?;
        if session.state != "open"
            || request.bytes().is_empty()
            || u64::try_from(request.bytes().len()).map_err(|_| invalid())?
                > self.capabilities.max_part_size().get()
            || &ChecksumSha256::digest_bytes(request.bytes()) != request.checksum_sha256()
        {
            return Err(invalid());
        }
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
        let upload = self
            .bucket
            .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
            .map_err(map_worker_error)?;
        let uploaded = upload
            .upload_part(request.part_number().get(), request.bytes().to_vec())
            .into_send()
            .await
            .map_err(map_worker_error)?;
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
                "INSERT INTO r2_multipart_parts_v1(upload_id,part_number,bytes,checksum_sha256,provider_etag,uploaded_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(upload_id,part_number) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&reference.upload_id().to_string()),
                JsValue::from_f64(f64::from(request.part_number().get())),
                number(receipt.size().get())?,
                JsValue::from_str(receipt.checksum_sha256().as_str()),
                JsValue::from_str(receipt.etag().expose_for_provider_comparison()),
                JsValue::from_f64(now()?.get() as f64),
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
        let reference = request.reference();
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, reference, &session)?;
        if let Some(completed) = self.completion(&request, context.correlation_id()).await? {
            return Ok(completed);
        }
        let stored_parts = self.parts(&session, context.correlation_id()).await?;
        if stored_parts != request.parts()
            || signed(request.expected_size().get())? != session.expected_bytes
            || request.expected_checksum_sha256().as_str() != session.checksum_sha256
            || request.expected_content_type().as_str() != session.content_type
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        self.database
            .prepare(
                "UPDATE r2_multipart_sessions_v1 SET state='completing' WHERE upload_id=?1 AND state='open'",
            )
            .bind(&[JsValue::from_str(&reference.upload_id().to_string())])
            .map_err(map_worker_error)?
            .run()
            .into_send()
            .await
            .map_err(map_worker_error)?;
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
            upload
                .complete(parts)
                .into_send()
                .await
                .map_err(map_worker_error)?;
        }
        let object = self
            .verify_full_object(
                reference.key(),
                request.expected_size(),
                request.expected_checksum_sha256(),
                request.expected_content_type(),
            )
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
        self.persist_completion(&request, &object, &probe, completed_at)
            .await?;
        self.completion(&request, context.correlation_id())
            .await?
            .ok_or_else(unavailable)
    }

    async fn abort_multipart(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        let session = self
            .session(reference.upload_id())
            .await?
            .ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, &session)?;
        let disposition = match session.state.as_str() {
            "complete" => ProviderAbortDispositionV1::AlreadyCompleted,
            "aborted" | "expired" => ProviderAbortDispositionV1::AlreadyAborted,
            "open" | "completing" => {
                let upload = self
                    .bucket
                    .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
                    .map_err(map_worker_error)?;
                upload.abort().into_send().await.map_err(map_worker_error)?;
                self.database
                    .prepare(
                        "UPDATE r2_multipart_sessions_v1 SET state='aborted' \
                         WHERE upload_id=?1 AND state IN ('open','completing')",
                    )
                    .bind(&[JsValue::from_str(&reference.upload_id().to_string())])
                    .map_err(map_worker_error)?
                    .run()
                    .into_send()
                    .await
                    .map_err(map_worker_error)?;
                ProviderAbortDispositionV1::Aborted
            }
            _ => return Err(integrity()),
        };
        Ok(ProviderAbortReceiptV1::new(
            reference.upload_id(),
            reference.key().clone(),
            disposition,
            context.correlation_id(),
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
    hex(value.as_bytes())
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

    #[test]
    fn provider_handles_are_opaque_safe_and_deterministic() {
        let raw = "r2/provider+upload=id";
        let encoded = encode_handle(raw);
        assert_eq!(encoded, "72322f70726f76696465722b75706c6f61643d6964");
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
}
