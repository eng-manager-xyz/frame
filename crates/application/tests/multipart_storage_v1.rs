use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};

use async_trait::async_trait;
use frame_application::{
    CreateMultipartCommandV1, IdempotentMultipartCommandV1, MultipartAuthorizationV1,
    MultipartGrantKeyMaterialV1, MultipartGrantKeyRingV1, MultipartReconcileOutcomeV1,
    MultipartStorageServiceV1, PrivateDownloadCommandV1, PrivateDownloadMethodV1,
    PrivateDownloadResponseV1, PrivateDownloadStatusV1, PutMultipartPartCommandV1,
    ScopedMultipartCommandV1,
};
use frame_domain::{
    AudioCodecV1, ByteSize, ChecksumSha256, ContentType, CorrelationId, CorsOriginV1,
    DownloadDispositionV1, DurationMillis, IdempotencyKey, MediaContainerV1, MultipartGrantId,
    MultipartGrantKeyVersion, MultipartGrantRecordV1, MultipartGrantScopeV1, MultipartGrantSecret,
    MultipartLimitsV1, MultipartOperationV1, MultipartPartNumberV1, MultipartUploadId,
    MultipartUploadSpecV1, ObjectRevision, ScopedObjectKey, StorageFileExtension, TenantId,
    TimestampMillis, TrustedMediaProbeV1, VideoCodecV1, VideoId, VideoObjectDescriptor,
};
use frame_ports::{
    DeterministicMultipartJournal, DeterministicMultipartObjectStore, DownloadPolicyV1,
    DownloadValidatorV1, JournalCreateOutcomeV1, JournalMutationOutcomeV1,
    MultipartJournalOperationV1, MultipartJournalPhaseV1, MultipartJournalV1,
    MultipartObjectStoreV1, MultipartProviderCapabilitiesV1, MultipartProviderOperationV1,
    MultipartReplayKeyV1, MultipartUploadSnapshotV1, ObjectByteRange, ObjectCachePolicy,
    ProviderAbortReceiptV1, ProviderCompleteMultipartRequestV1, ProviderCompletedObjectV1,
    ProviderCreateMultipartRequestV1, ProviderDownloadBodyV1, ProviderDownloadMetadataV1,
    ProviderDownloadRequestV1, ProviderDownloadResponseV1, ProviderEntityTag,
    ProviderLookupMultipartRequestV1, ProviderMultipartHandleV1, ProviderMultipartSessionV1,
    ProviderObjectVersion, ProviderPartReceiptV1, ProviderPartsListV1, ProviderPutPartRequestV1,
    ProviderUploadReferenceV1, SourceFinalizeRecordV1, StorageFailure, StorageFailureKind,
    StorageRequestContext,
};

const ATTACK_NONE: u8 = 0;
const ATTACK_CREATE: u8 = 1;
const ATTACK_LIST: u8 = 2;
const ATTACK_PART: u8 = 3;
const ATTACK_COMPLETE: u8 = 4;
const ATTACK_ABORT: u8 = 5;
const ATTACK_DOWNLOAD: u8 = 6;
const ATTACK_DOWNLOAD_SIZE: u8 = 7;
const ATTACK_DOWNLOAD_CHECKSUM: u8 = 8;
const ATTACK_DOWNLOAD_CONTENT_TYPE: u8 = 9;
const ATTACK_DOWNLOAD_VERSION: u8 = 10;
const ATTACK_DOWNLOAD_ETAG: u8 = 11;
const ATTACK_DOWNLOAD_LAST_MODIFIED: u8 = 12;
const ATTACK_BODY_EMPTY: u8 = 20;
const ATTACK_BODY_OVERSIZED: u8 = 21;
const ATTACK_BODY_EARLY_EOF: u8 = 22;
const ATTACK_BODY_EXTRA: u8 = 23;
const ATTACK_BODY_MIDSTREAM: u8 = 24;
const ATTACK_BODY_CORRUPT: u8 = 25;
const ATTACK_BODY_TRACK_DROP: u8 = 26;
const JOURNAL_ATTACK_NONE: u8 = 0;
const JOURNAL_ATTACK_REGISTER: u8 = 1;
const JOURNAL_ATTACK_CLAIM: u8 = 2;
const JOURNAL_ATTACK_ACTIVATE: u8 = 3;
const JOURNAL_ATTACK_PART: u8 = 4;
const JOURNAL_ATTACK_COMPLETE: u8 = 5;
const JOURNAL_ATTACK_FINALIZE: u8 = 6;
const JOURNAL_ATTACK_ABORT: u8 = 7;
const JOURNAL_ATTACK_GET_GRANT: u8 = 8;
const JOURNAL_ATTACK_FINALIZE_INPUT: u8 = 9;

struct AdversarialDownloadBody {
    inner: Box<dyn ProviderDownloadBodyV1>,
    attack: u8,
    pulls: u8,
    emitted_extra: bool,
    drop_count: Arc<AtomicUsize>,
}

#[async_trait]
impl ProviderDownloadBodyV1 for AdversarialDownloadBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure> {
        let pull = self.pulls;
        self.pulls = self.pulls.saturating_add(1);
        match self.attack {
            ATTACK_BODY_EMPTY if pull == 0 => Ok(Some(Vec::new())),
            ATTACK_BODY_OVERSIZED if pull == 0 => Ok(Some(vec![0; 11])),
            ATTACK_BODY_EARLY_EOF => Ok(None),
            ATTACK_BODY_MIDSTREAM if pull > 0 => {
                Err(StorageFailure::new(StorageFailureKind::Unavailable))
            }
            ATTACK_BODY_EXTRA => match self.inner.next_chunk().await? {
                Some(chunk) => Ok(Some(chunk)),
                None if !self.emitted_extra => {
                    self.emitted_extra = true;
                    Ok(Some(vec![0xff]))
                }
                None => Ok(None),
            },
            ATTACK_BODY_CORRUPT => {
                let mut chunk = self.inner.next_chunk().await?;
                if pull == 0
                    && let Some(bytes) = &mut chunk
                    && let Some(first) = bytes.first_mut()
                {
                    *first ^= 0xff;
                }
                Ok(chunk)
            }
            _ => self.inner.next_chunk().await,
        }
    }
}

impl Drop for AdversarialDownloadBody {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::Relaxed);
    }
}

struct CorruptingMultipartProvider {
    inner: DeterministicMultipartObjectStore,
    wrong_key: ScopedObjectKey,
    attack: AtomicU8,
    complete_calls: AtomicUsize,
    body_drop_count: Arc<AtomicUsize>,
}

impl CorruptingMultipartProvider {
    fn new(wrong_key: ScopedObjectKey) -> Self {
        Self {
            inner: DeterministicMultipartObjectStore::new(capabilities(), probe()),
            wrong_key,
            attack: AtomicU8::new(ATTACK_NONE),
            complete_calls: AtomicUsize::new(0),
            body_drop_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn attack(&self, value: u8) {
        self.attack.store(value, Ordering::Relaxed);
    }

    fn body_drop_count(&self) -> usize {
        self.body_drop_count.load(Ordering::Relaxed)
    }

    fn complete_calls(&self) -> usize {
        self.complete_calls.load(Ordering::Relaxed)
    }

    fn corrupt_metadata(
        &self,
        metadata: &ProviderDownloadMetadataV1,
    ) -> ProviderDownloadMetadataV1 {
        let mut key = metadata.key().clone();
        let mut object_size = metadata.size();
        let mut checksum = metadata.checksum_sha256().clone();
        let mut content_type = metadata.content_type().clone();
        let mut version = metadata.provider_version().clone();
        let mut etag = metadata.provider_etag().clone();
        let mut last_modified = metadata.last_modified();
        match self.attack.load(Ordering::Relaxed) {
            ATTACK_DOWNLOAD => key = self.wrong_key.clone(),
            ATTACK_DOWNLOAD_SIZE => object_size = size(object_size.get() + 1),
            ATTACK_DOWNLOAD_CHECKSUM => {
                checksum = ChecksumSha256::parse("a".repeat(64)).expect("checksum")
            }
            ATTACK_DOWNLOAD_CONTENT_TYPE => {
                content_type = ContentType::parse("video/mp4").expect("content type")
            }
            ATTACK_DOWNLOAD_VERSION => {
                version = ProviderObjectVersion::parse("wrong-provider-version").expect("version")
            }
            ATTACK_DOWNLOAD_ETAG => {
                etag = ProviderEntityTag::parse("wrong-provider-etag").expect("etag")
            }
            ATTACK_DOWNLOAD_LAST_MODIFIED => last_modified = timestamp(last_modified.get() + 1),
            _ => {}
        }
        ProviderDownloadMetadataV1::new(
            key,
            object_size,
            checksum,
            content_type,
            version,
            etag,
            last_modified,
            metadata.correlation_id(),
        )
    }

    fn corrupt_download(&self, response: ProviderDownloadResponseV1) -> ProviderDownloadResponseV1 {
        match response {
            ProviderDownloadResponseV1::NotModified(metadata) => {
                ProviderDownloadResponseV1::NotModified(self.corrupt_metadata(&metadata))
            }
            ProviderDownloadResponseV1::Head(metadata) => {
                ProviderDownloadResponseV1::Head(self.corrupt_metadata(&metadata))
            }
            ProviderDownloadResponseV1::Body {
                metadata,
                range,
                body,
            } => ProviderDownloadResponseV1::Body {
                metadata: self.corrupt_metadata(&metadata),
                range,
                body: if self.attack.load(Ordering::Relaxed) >= ATTACK_BODY_EMPTY {
                    Box::new(AdversarialDownloadBody {
                        inner: body,
                        attack: self.attack.load(Ordering::Relaxed),
                        pulls: 0,
                        emitted_extra: false,
                        drop_count: Arc::clone(&self.body_drop_count),
                    })
                } else {
                    body
                },
            },
        }
    }
}

#[async_trait]
impl MultipartObjectStoreV1 for CorruptingMultipartProvider {
    fn capabilities(&self) -> MultipartProviderCapabilitiesV1 {
        self.inner.capabilities()
    }

    async fn create_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCreateMultipartRequestV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        let session = self.inner.create_multipart(context, request).await?;
        if self.attack.load(Ordering::Relaxed) == ATTACK_CREATE {
            Ok(ProviderMultipartSessionV1::new(
                session.upload_id(),
                self.wrong_key.clone(),
                session.handle().clone(),
                session.expires_at(),
                session.correlation_id(),
            ))
        } else {
            Ok(session)
        }
    }

    async fn lookup_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderLookupMultipartRequestV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure> {
        self.inner.lookup_multipart(context, request).await
    }

    async fn list_parts(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderPartsListV1, StorageFailure> {
        let list = self.inner.list_parts(context, reference).await?;
        if self.attack.load(Ordering::Relaxed) == ATTACK_LIST {
            ProviderPartsListV1::new(
                list.upload_id(),
                self.wrong_key.clone(),
                Vec::new(),
                list.correlation_id(),
            )
        } else {
            Ok(list)
        }
    }

    async fn put_part(
        &self,
        context: StorageRequestContext,
        request: ProviderPutPartRequestV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        let receipt = self.inner.put_part(context, request).await?;
        if self.attack.load(Ordering::Relaxed) == ATTACK_PART {
            Ok(ProviderPartReceiptV1::new(
                receipt.upload_id(),
                receipt.key().clone(),
                receipt.part_number(),
                receipt.size(),
                ChecksumSha256::parse("f".repeat(64)).expect("checksum"),
                receipt.etag().clone(),
                receipt.correlation_id(),
            ))
        } else {
            Ok(receipt)
        }
    }

    async fn complete_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCompleteMultipartRequestV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        self.complete_calls.fetch_add(1, Ordering::Relaxed);
        let completed = self.inner.complete_multipart(context, request).await?;
        if self.attack.load(Ordering::Relaxed) == ATTACK_COMPLETE {
            Ok(ProviderCompletedObjectV1::new(
                completed.upload_id(),
                self.wrong_key.clone(),
                completed.size(),
                completed.checksum_sha256().clone(),
                completed.content_type().clone(),
                completed.provider_version().clone(),
                completed.provider_etag().clone(),
                completed.last_modified(),
                completed.media_probe().clone(),
                completed.correlation_id(),
            ))
        } else {
            Ok(completed)
        }
    }

    async fn abort_multipart(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        let receipt = self.inner.abort_multipart(context, reference).await?;
        if self.attack.load(Ordering::Relaxed) == ATTACK_ABORT {
            Ok(ProviderAbortReceiptV1::new(
                receipt.upload_id(),
                self.wrong_key.clone(),
                receipt.disposition(),
                receipt.correlation_id(),
            ))
        } else {
            Ok(receipt)
        }
    }

    async fn head_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        let response = self.inner.head_private(context, request).await?;
        if self.attack.load(Ordering::Relaxed) >= ATTACK_DOWNLOAD {
            Ok(self.corrupt_download(response))
        } else {
            Ok(response)
        }
    }

    async fn get_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        let response = self.inner.get_private(context, request).await?;
        if self.attack.load(Ordering::Relaxed) >= ATTACK_DOWNLOAD {
            Ok(self.corrupt_download(response))
        } else {
            Ok(response)
        }
    }
}

struct CorruptingMultipartJournal {
    inner: DeterministicMultipartJournal,
    wrong_key: ScopedObjectKey,
    attack: AtomicU8,
    finalize_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
}

impl CorruptingMultipartJournal {
    fn new(wrong_key: ScopedObjectKey) -> Self {
        Self {
            inner: DeterministicMultipartJournal::default(),
            wrong_key,
            attack: AtomicU8::new(JOURNAL_ATTACK_NONE),
            finalize_barrier: Mutex::new(None),
        }
    }

    fn attack(&self, value: u8) {
        self.attack.store(value, Ordering::Relaxed);
    }

    fn set_finalize_barrier(&self, barrier: Option<Arc<tokio::sync::Barrier>>) {
        *self.finalize_barrier.lock().expect("finalize barrier lock") = barrier;
    }

    fn finalize_barrier(&self) -> Option<Arc<tokio::sync::Barrier>> {
        self.finalize_barrier
            .lock()
            .expect("finalize barrier lock")
            .clone()
    }

    fn corrupt_part(&self, receipt: &ProviderPartReceiptV1) -> ProviderPartReceiptV1 {
        ProviderPartReceiptV1::new(
            receipt.upload_id(),
            receipt.key().clone(),
            receipt.part_number(),
            receipt.size(),
            ChecksumSha256::parse("e".repeat(64)).expect("checksum"),
            receipt.etag().clone(),
            receipt.correlation_id(),
        )
    }

    fn corrupt_completed(
        &self,
        completed: &ProviderCompletedObjectV1,
    ) -> ProviderCompletedObjectV1 {
        ProviderCompletedObjectV1::new(
            completed.upload_id(),
            self.wrong_key.clone(),
            completed.size(),
            completed.checksum_sha256().clone(),
            completed.content_type().clone(),
            completed.provider_version().clone(),
            completed.provider_etag().clone(),
            completed.last_modified(),
            completed.media_probe().clone(),
            completed.correlation_id(),
        )
    }

    fn corrupt_finalize(&self, record: &SourceFinalizeRecordV1) -> SourceFinalizeRecordV1 {
        SourceFinalizeRecordV1::new(
            record.upload_id(),
            record.key().clone(),
            record.provider_version().clone(),
            record.provider_etag().clone(),
            record.size(),
            record.checksum_sha256().clone(),
            record.content_type().clone(),
            record.provider_last_modified(),
            record.media_probe().clone(),
            TimestampMillis::new(record.finalized_at().get() + 1).expect("timestamp"),
            record.correlation_id(),
        )
    }
}

#[async_trait]
impl MultipartJournalV1 for CorruptingMultipartJournal {
    async fn register_grant(
        &self,
        context: StorageRequestContext,
        record: MultipartGrantRecordV1,
    ) -> Result<JournalMutationOutcomeV1<MultipartGrantRecordV1>, StorageFailure> {
        let outcome = self.inner.register_grant(context, record).await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_REGISTER {
            return Ok(outcome);
        }
        let mut record = match outcome {
            JournalMutationOutcomeV1::Applied(record)
            | JournalMutationOutcomeV1::Replay(record) => record,
        };
        record
            .revoke(record.issued_at())
            .expect("valid injected revocation");
        Ok(JournalMutationOutcomeV1::Applied(record))
    }

    async fn get_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
    ) -> Result<Option<MultipartGrantRecordV1>, StorageFailure> {
        let record = self.inner.get_grant(context, id).await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_GET_GRANT {
            return Ok(record);
        }
        let Some(record) = record else {
            return Ok(None);
        };
        let mut corrupt = MultipartGrantRecordV1::active(
            MultipartGrantId::new(),
            record.digest().clone(),
            record.key_version(),
            record.scope().clone(),
            record.issued_at(),
            record.expires_at(),
        )
        .expect("valid mismatched grant id");
        if let Some(revoked_at) = record.revoked_at() {
            corrupt.revoke(revoked_at).expect("valid revocation");
        }
        Ok(Some(corrupt))
    }

    async fn revoke_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        self.inner.revoke_grant(context, id, revoked_at).await
    }

    async fn claim_create(
        &self,
        context: StorageRequestContext,
        grant_id: MultipartGrantId,
        now: TimestampMillis,
        idempotency_key: IdempotencyKey,
        fingerprint: ChecksumSha256,
        draft: MultipartUploadSnapshotV1,
    ) -> Result<JournalCreateOutcomeV1, StorageFailure> {
        let outcome = self
            .inner
            .claim_create(context, grant_id, now, idempotency_key, fingerprint, draft)
            .await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_CLAIM {
            return Ok(outcome);
        }
        let snapshot = match outcome {
            JournalCreateOutcomeV1::Claimed(snapshot)
            | JournalCreateOutcomeV1::Resume(snapshot)
            | JournalCreateOutcomeV1::Replay(snapshot) => snapshot,
        };
        Ok(JournalCreateOutcomeV1::Claimed(
            MultipartUploadSnapshotV1::new(
                snapshot.upload_id(),
                snapshot.spec().clone(),
                None,
                Vec::new(),
                MultipartJournalPhaseV1::Uploading,
                None,
                snapshot.expires_at(),
                snapshot.correlation_id(),
            ),
        ))
    }

    async fn activate_upload(
        &self,
        context: StorageRequestContext,
        session: ProviderMultipartSessionV1,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure> {
        let snapshot = self.inner.activate_upload(context, session).await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_ACTIVATE {
            return Ok(snapshot);
        }
        Ok(MultipartUploadSnapshotV1::new(
            snapshot.upload_id(),
            snapshot.spec().clone(),
            snapshot.provider_session().cloned(),
            snapshot.parts().to_vec(),
            MultipartJournalPhaseV1::Creating,
            snapshot.completed().cloned(),
            snapshot.expires_at(),
            snapshot.correlation_id(),
        ))
    }

    async fn get_upload(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<MultipartUploadSnapshotV1>, StorageFailure> {
        self.inner.get_upload(context, upload_id).await
    }

    async fn get_finalize(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        self.inner.get_finalize(context, upload_id).await
    }

    async fn get_finalize_by_key(
        &self,
        context: StorageRequestContext,
        key: ScopedObjectKey,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        self.inner.get_finalize_by_key(context, key).await
    }

    async fn record_part(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderPartReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderPartReceiptV1>, StorageFailure> {
        let outcome = self
            .inner
            .record_part(context, replay_key, fingerprint, receipt)
            .await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_PART {
            return Ok(outcome);
        }
        let receipt = match outcome {
            JournalMutationOutcomeV1::Applied(receipt)
            | JournalMutationOutcomeV1::Replay(receipt) => receipt,
        };
        Ok(JournalMutationOutcomeV1::Applied(
            self.corrupt_part(&receipt),
        ))
    }

    async fn record_provider_complete(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        completed: ProviderCompletedObjectV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderCompletedObjectV1>, StorageFailure> {
        let outcome = self
            .inner
            .record_provider_complete(context, replay_key, fingerprint, completed)
            .await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_COMPLETE {
            return Ok(outcome);
        }
        let completed = match outcome {
            JournalMutationOutcomeV1::Applied(completed)
            | JournalMutationOutcomeV1::Replay(completed) => completed,
        };
        Ok(JournalMutationOutcomeV1::Applied(
            self.corrupt_completed(&completed),
        ))
    }

    async fn finalize(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        record: SourceFinalizeRecordV1,
    ) -> Result<JournalMutationOutcomeV1<SourceFinalizeRecordV1>, StorageFailure> {
        if let Some(barrier) = self.finalize_barrier() {
            barrier.wait().await;
        }
        let record = if self.attack.load(Ordering::Relaxed) == JOURNAL_ATTACK_FINALIZE_INPUT {
            SourceFinalizeRecordV1::new(
                record.upload_id(),
                record.key().clone(),
                record.provider_version().clone(),
                record.provider_etag().clone(),
                record.size(),
                record.checksum_sha256().clone(),
                record.content_type().clone(),
                record.provider_last_modified(),
                record.media_probe().clone(),
                timestamp(0),
                record.correlation_id(),
            )
        } else {
            record
        };
        let outcome = self
            .inner
            .finalize(context, replay_key, fingerprint, record)
            .await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_FINALIZE {
            return Ok(outcome);
        }
        let record = match outcome {
            JournalMutationOutcomeV1::Applied(record)
            | JournalMutationOutcomeV1::Replay(record) => record,
        };
        Ok(JournalMutationOutcomeV1::Applied(
            self.corrupt_finalize(&record),
        ))
    }

    async fn abort(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderAbortReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderAbortReceiptV1>, StorageFailure> {
        let outcome = self
            .inner
            .abort(context, replay_key, fingerprint, receipt)
            .await?;
        if self.attack.load(Ordering::Relaxed) != JOURNAL_ATTACK_ABORT {
            return Ok(outcome);
        }
        let receipt = match outcome {
            JournalMutationOutcomeV1::Applied(receipt)
            | JournalMutationOutcomeV1::Replay(receipt) => receipt,
        };
        Ok(JournalMutationOutcomeV1::Applied(
            ProviderAbortReceiptV1::new(
                receipt.upload_id(),
                self.wrong_key.clone(),
                receipt.disposition(),
                receipt.correlation_id(),
            ),
        ))
    }

    async fn reconciliation_candidates(
        &self,
        context: StorageRequestContext,
        limit: u16,
    ) -> Result<Vec<MultipartUploadSnapshotV1>, StorageFailure> {
        self.inner.reconciliation_candidates(context, limit).await
    }
}

fn size(value: u64) -> ByteSize {
    ByteSize::new(value).expect("size")
}

fn timestamp(value: i64) -> TimestampMillis {
    TimestampMillis::new(value).expect("timestamp")
}

fn limits() -> MultipartLimitsV1 {
    MultipartLimitsV1::new(
        size(5),
        size(10),
        10,
        size(100),
        size(10),
        DurationMillis::new(1_000).expect("ttl"),
    )
    .expect("limits")
}

fn capabilities() -> MultipartProviderCapabilitiesV1 {
    MultipartProviderCapabilitiesV1::full(size(5), size(10), 10, size(100), size(10), true)
        .expect("capabilities")
}

fn probe() -> TrustedMediaProbeV1 {
    TrustedMediaProbeV1::new(
        MediaContainerV1::Webm,
        VideoCodecV1::Vp9,
        AudioCodecV1::Opus,
        1920,
        1080,
        30_000,
        30_000,
    )
    .expect("probe")
}

fn source_key(tenant: TenantId, video: VideoId, revision: u64) -> ScopedObjectKey {
    ScopedObjectKey::source(
        tenant,
        video,
        ObjectRevision::new(revision).expect("revision"),
        VideoObjectDescriptor::Source {
            extension: StorageFileExtension::parse("webm").expect("extension"),
        },
    )
    .expect("source key")
}

fn spec(key: ScopedObjectKey, bytes: &[u8]) -> MultipartUploadSpecV1 {
    MultipartUploadSpecV1::new(
        key,
        size(u64::try_from(bytes.len()).expect("length")),
        size(10),
        ChecksumSha256::digest_bytes(bytes),
        ContentType::parse("video/webm").expect("content type"),
        limits(),
    )
    .expect("spec")
}

fn context(tenant: TenantId) -> StorageRequestContext {
    StorageRequestContext::new(tenant, CorrelationId::new())
}

fn build_service<'a>(
    provider: &'a dyn MultipartObjectStoreV1,
    journal: &'a dyn MultipartJournalV1,
) -> MultipartStorageServiceV1<'a> {
    let version = MultipartGrantKeyVersion::new(2).expect("version");
    let keys = MultipartGrantKeyRingV1::new(
        version,
        [(
            version,
            MultipartGrantKeyMaterialV1::parse(vec![0x5a; 32]).expect("key"),
        )],
    )
    .expect("key ring");
    MultipartStorageServiceV1::new(
        provider,
        journal,
        keys,
        limits(),
        DownloadPolicyV1::new(
            vec![CorsOriginV1::parse("https://app.example.com").expect("origin")],
            ObjectCachePolicy::PrivateImmutable,
        )
        .expect("download policy"),
    )
}

fn build_service_with_key_versions<'a>(
    provider: &'a dyn MultipartObjectStoreV1,
    journal: &'a dyn MultipartJournalV1,
    active: u16,
    versions: &[u16],
) -> MultipartStorageServiceV1<'a> {
    let active = MultipartGrantKeyVersion::new(active).expect("active version");
    let keys = versions
        .iter()
        .copied()
        .map(|version| {
            (
                MultipartGrantKeyVersion::new(version).expect("version"),
                MultipartGrantKeyMaterialV1::parse(vec![
                    u8::try_from(version)
                        .expect("test version fits in u8");
                    32
                ])
                .expect("key"),
            )
        })
        .collect::<Vec<_>>();
    MultipartStorageServiceV1::new(
        provider,
        journal,
        MultipartGrantKeyRingV1::new(active, keys).expect("key ring"),
        limits(),
        DownloadPolicyV1::new(Vec::new(), ObjectCachePolicy::PrivateImmutable)
            .expect("download policy"),
    )
}

fn secret() -> MultipartGrantSecret {
    MultipartGrantSecret::parse("multipart-test-secret-material-0001").expect("secret")
}

async fn grant(
    service: &MultipartStorageServiceV1<'_>,
    context: StorageRequestContext,
    key: ScopedObjectKey,
    upload_id: Option<frame_domain::MultipartUploadId>,
    operation: MultipartOperationV1,
) -> MultipartAuthorizationV1 {
    let secret = secret();
    let id = MultipartGrantId::new();
    let scope =
        MultipartGrantScopeV1::new(context.tenant_id(), key, upload_id, operation).expect("scope");
    service
        .register_grant(
            context,
            id,
            &secret,
            MultipartGrantKeyVersion::new(2).expect("version"),
            scope,
            timestamp(1),
            timestamp(1_000),
        )
        .await
        .expect("register grant");
    MultipartAuthorizationV1::new(id, secret)
}

fn idem(value: &str) -> IdempotencyKey {
    IdempotencyKey::parse(value).expect("idempotency key")
}

async fn collect_body(response: &mut PrivateDownloadResponseV1) -> Vec<u8> {
    let body = response.body_mut().expect("download body");
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await.expect("valid download chunk") {
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

async fn collect_until_body_error(
    response: &mut PrivateDownloadResponseV1,
) -> (Vec<u8>, StorageFailure) {
    let body = response.body_mut().expect("download body");
    let mut bytes = Vec::new();
    loop {
        match body.next_chunk().await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) => panic!("expected a body validation error"),
            Err(error) => return (bytes, error),
        }
    }
}

fn scoped(
    authorization: MultipartAuthorizationV1,
    upload_id: frame_domain::MultipartUploadId,
    key: ScopedObjectKey,
    now: i64,
) -> ScopedMultipartCommandV1 {
    ScopedMultipartCommandV1::new(authorization, upload_id, key, timestamp(now))
}

async fn prepare_provider_completed_upload(
    service: &MultipartStorageServiceV1<'_>,
    context: StorageRequestContext,
    key: ScopedObjectKey,
    bytes: &[u8],
) -> MultipartUploadId {
    let create_auth = grant(
        service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth,
                idem("finalize-race-create"),
                spec(key.clone(), bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create finalize-race upload");
    let put_auth = grant(
        service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;
    service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth, plan.upload_id(), key.clone(), 20),
                idem("finalize-race-part"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(bytes),
                bytes.to_vec(),
            ),
        )
        .await
        .expect("put finalize-race part");
    let complete_auth = grant(
        service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Complete,
    )
    .await;
    service
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth, plan.upload_id(), key, 30),
                idem("finalize-race-complete"),
            ),
        )
        .await
        .expect("complete finalize-race upload");
    plan.upload_id()
}

fn assert_same_durable_finalize(left: &SourceFinalizeRecordV1, right: &SourceFinalizeRecordV1) {
    assert_eq!(left.upload_id(), right.upload_id());
    assert_eq!(left.key(), right.key());
    assert_eq!(left.provider_version(), right.provider_version());
    assert_eq!(left.provider_etag(), right.provider_etag());
    assert_eq!(left.size(), right.size());
    assert_eq!(left.checksum_sha256(), right.checksum_sha256());
    assert_eq!(left.content_type(), right.content_type());
    assert_eq!(
        left.provider_last_modified(),
        right.provider_last_modified()
    );
    assert_eq!(left.media_probe(), right.media_probe());
    assert_eq!(left.finalized_at(), right.finalized_at());
}

#[tokio::test]
async fn restart_resume_reconciliation_finalize_and_private_playback_are_safe() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let key = source_key(tenant, video, 1);
    let bytes = b"0123456789abcdefghijXYZ";
    assert_eq!(bytes.len(), 23);
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = DeterministicMultipartJournal::default();
    let service = build_service(&provider, &journal);

    let create_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let upload_spec = spec(key.clone(), bytes);
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth.clone(),
                idem("create-recording-0001"),
                upload_spec.clone(),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create");
    let replay = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth.clone(),
                idem("create-recording-0001"),
                upload_spec.clone(),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create replay");
    assert_eq!(plan.upload_id(), replay.upload_id());
    assert_eq!(plan.handle(), replay.handle());
    assert_eq!(plan.part_count(), 3);
    let create_retry_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let create_rebound = service
        .create(
            create_retry_context,
            CreateMultipartCommandV1::new(
                create_auth.clone(),
                idem("create-recording-0001"),
                upload_spec.clone(),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create replay with fresh correlation");
    assert_eq!(
        create_rebound.correlation_id(),
        create_retry_context.correlation_id()
    );
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    create_auth.clone(),
                    idem("create-recording-0002"),
                    upload_spec.clone(),
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("create grant cannot claim a second upload")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    let changed_spec = MultipartUploadSpecV1::new(
        key.clone(),
        upload_spec.total_size(),
        upload_spec.part_size(),
        ChecksumSha256::parse("d".repeat(64)).expect("checksum"),
        upload_spec.content_type().clone(),
        limits(),
    )
    .expect("changed spec");
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    create_auth,
                    idem("create-recording-0001"),
                    changed_spec,
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("changed create replay")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );

    let put_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;
    let part_one = &bytes[..10];
    let first_receipt = service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                idem("put-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(part_one),
                part_one.to_vec(),
            ),
        )
        .await
        .expect("part one");
    let first_replay = service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                idem("put-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(part_one),
                part_one.to_vec(),
            ),
        )
        .await
        .expect("part replay");
    assert_eq!(first_receipt, first_replay);
    let changed_part = b"XXXXXXXXXX";
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                    idem("put-part-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(changed_part),
                    changed_part.to_vec(),
                ),
            )
            .await
            .expect_err("changed part replay")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    let retry_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let rebound = service
        .put_part(
            retry_context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                idem("put-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(part_one),
                part_one.to_vec(),
            ),
        )
        .await
        .expect("semantic replay with a fresh correlation");
    assert_eq!(rebound.correlation_id(), retry_context.correlation_id());

    let part_three = &bytes[20..];
    journal
        .inject_failure(
            MultipartJournalOperationV1::RecordPart,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("inject part journal failure");
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), key.clone(), 21),
                    idem("put-part-0003"),
                    MultipartPartNumberV1::new(3).expect("part"),
                    ChecksumSha256::digest_bytes(part_three),
                    part_three.to_vec(),
                ),
            )
            .await
            .expect_err("part journal failure")
            .kind(),
        StorageFailureKind::Unavailable
    );
    service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth.clone(), plan.upload_id(), key.clone(), 21),
                idem("put-part-0003"),
                MultipartPartNumberV1::new(3).expect("part"),
                ChecksumSha256::digest_bytes(part_three),
                part_three.to_vec(),
            ),
        )
        .await
        .expect("sparse part three");

    // A fresh service instance reconstructs all server state from the journal/provider, while the
    // caller can rebuild its local journal from provider-verified sparse parts.
    let restarted = build_service(&provider, &journal);
    let list_auth = grant(
        &restarted,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::ListParts,
    )
    .await;
    let resume = restarted
        .list_parts(
            context,
            scoped(list_auth, plan.upload_id(), key.clone(), 30),
        )
        .await
        .expect("resume list");
    assert_eq!(resume.verified_parts().len(), 2);
    assert_eq!(resume.verified_parts()[0].part_number().get(), 1);
    assert_eq!(resume.verified_parts()[1].part_number().get(), 3);

    let part_two = &bytes[10..20];
    restarted
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth, plan.upload_id(), key.clone(), 31),
                idem("put-part-0002"),
                MultipartPartNumberV1::new(2).expect("part"),
                ChecksumSha256::digest_bytes(part_two),
                part_two.to_vec(),
            ),
        )
        .await
        .expect("part two");

    let complete_auth = grant(
        &restarted,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Complete,
    )
    .await;
    journal
        .inject_failure(
            MultipartJournalOperationV1::RecordComplete,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("inject journal failure");
    let first_complete = restarted
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth.clone(), plan.upload_id(), key.clone(), 40),
                idem("complete-upload-0001"),
            ),
        )
        .await
        .expect_err("provider completion and journal commit are deliberately non-atomic");
    assert_eq!(first_complete.kind(), StorageFailureKind::Unavailable);
    assert_eq!(provider.object_count().expect("object count"), 1);
    let completed = restarted
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth.clone(), plan.upload_id(), key.clone(), 40),
                idem("complete-upload-0001"),
            ),
        )
        .await
        .expect("complete recovery");
    assert_eq!(
        completed.checksum_sha256(),
        &ChecksumSha256::digest_bytes(bytes)
    );
    assert_eq!(completed.media_probe(), &probe());
    let complete_retry_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let complete_rebound = restarted
        .complete(
            complete_retry_context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth.clone(), plan.upload_id(), key.clone(), 40),
                idem("complete-upload-0001"),
            ),
        )
        .await
        .expect("complete replay with fresh correlation");
    assert_eq!(
        complete_rebound.correlation_id(),
        complete_retry_context.correlation_id()
    );
    let textual_system_poison = format!("reconcile-finalize-{}", plan.upload_id());
    restarted
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth.clone(), plan.upload_id(), key.clone(), 40),
                idem(&textual_system_poison),
            ),
        )
        .await
        .expect("client text is isolated from the system namespace");

    let finalize_auth = grant(
        &restarted,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Finalize,
    )
    .await;
    journal
        .inject_failure(
            MultipartJournalOperationV1::Finalize,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("inject finalize failure");
    assert_eq!(
        restarted
            .finalize(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(finalize_auth.clone(), plan.upload_id(), key.clone(), 5),
                    idem("finalize-too-early"),
                ),
            )
            .await
            .expect_err("finalization cannot predate provider last-modified")
            .kind(),
        StorageFailureKind::Integrity
    );
    assert_eq!(
        restarted
            .finalize(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(finalize_auth.clone(), plan.upload_id(), key.clone(), 50),
                    idem("finalize-upload-0001"),
                ),
            )
            .await
            .expect_err("finalize failure")
            .kind(),
        StorageFailureKind::Unavailable
    );
    let candidates = restarted
        .reconciliation_candidates(context, 10)
        .await
        .expect("candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].phase(),
        MultipartJournalPhaseV1::ProviderCompleted
    );
    let MultipartReconcileOutcomeV1::Finalized(reconciled) = restarted
        .reconcile(context, plan.upload_id(), timestamp(51))
        .await
        .expect("reconcile")
    else {
        panic!("expected finalized reconciliation");
    };
    assert_eq!(reconciled.provider_version(), completed.provider_version());
    assert_eq!(reconciled.media_probe(), completed.media_probe());
    let finalize_replay = restarted
        .finalize(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth.clone(), plan.upload_id(), key.clone(), 52),
                idem("finalize-upload-0001"),
            ),
        )
        .await
        .expect("finalize replay");
    assert_eq!(finalize_replay.finalized_at(), timestamp(51));
    let finalize_retry_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let finalize_rebound = restarted
        .finalize(
            finalize_retry_context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth, plan.upload_id(), key.clone(), 52),
                idem("finalize-upload-0001"),
            ),
        )
        .await
        .expect("finalize replay with fresh correlation");
    assert_eq!(
        finalize_rebound.correlation_id(),
        finalize_retry_context.correlation_id()
    );
    assert_eq!(
        restarted
            .complete(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(complete_auth, plan.upload_id(), key.clone(), 52),
                    idem("finalize-upload-0001"),
                ),
            )
            .await
            .expect_err("terminal complete still checks replay operation")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );

    let origin = CorsOriginV1::parse("https://app.example.com").expect("origin");
    let head_auth = grant(
        &restarted,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Head,
    )
    .await;
    let head = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                head_auth,
                key.clone(),
                PrivateDownloadMethodV1::Head,
                None,
                DownloadValidatorV1::None,
                Some(origin.clone()),
                DownloadDispositionV1::Inline,
                timestamp(60),
            ),
        )
        .await
        .expect("head");
    assert_eq!(head.status(), PrivateDownloadStatusV1::Ok);
    assert_eq!(head.headers().content_length(), Some(size(23)));
    assert!(head.headers().accept_ranges());
    assert_eq!(head.headers().cors_allow_origin(), Some(&origin));
    assert!(!head.has_body());

    let range_auth = grant(
        &restarted,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Range,
    )
    .await;
    let range = ObjectByteRange::new(2, 9).expect("range");
    let mut partial = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                range_auth,
                key.clone(),
                PrivateDownloadMethodV1::Get,
                Some(range),
                DownloadValidatorV1::IfMatch(head.headers().etag().clone()),
                Some(origin.clone()),
                DownloadDispositionV1::Inline,
                timestamp(61),
            ),
        )
        .await
        .expect("range get");
    assert_eq!(partial.status(), PrivateDownloadStatusV1::PartialContent);
    let content_range = partial.headers().content_range().expect("content range");
    assert_eq!(content_range.range(), range);
    assert_eq!(content_range.total_size(), size(23));
    assert_eq!(collect_body(&mut partial).await, bytes[2..9]);

    let get_auth = grant(
        &restarted,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Get,
    )
    .await;
    let mut full = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth.clone(),
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::None,
                None,
                DownloadDispositionV1::Attachment,
                timestamp(62),
            ),
        )
        .await
        .expect("full get");
    assert_eq!(full.status(), PrivateDownloadStatusV1::Ok);
    assert_eq!(collect_body(&mut full).await, bytes);
    assert_eq!(
        full.headers().disposition(),
        DownloadDispositionV1::Attachment
    );
    assert_eq!(full.headers().content_type().as_str(), "video/webm");
    assert_eq!(
        full.headers().cache_policy(),
        ObjectCachePolicy::PrivateImmutable
    );
    let not_modified = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth.clone(),
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::IfNoneMatch(head.headers().etag().clone()),
                Some(origin),
                DownloadDispositionV1::Inline,
                timestamp(62),
            ),
        )
        .await
        .expect("conditional get");
    assert_eq!(not_modified.status(), PrivateDownloadStatusV1::NotModified);
    assert!(not_modified.headers().content_length().is_none());
    assert!(!not_modified.has_body());

    let failed_precondition = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth.clone(),
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::IfMatch(ProviderEntityTag::parse("wrong-etag").expect("etag")),
                None,
                DownloadDispositionV1::Inline,
                timestamp(63),
            ),
        )
        .await
        .expect_err("if-match failure");
    assert_eq!(
        failed_precondition.kind(),
        StorageFailureKind::PreconditionFailed
    );

    let disallowed = restarted
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth,
                key,
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::None,
                Some(CorsOriginV1::parse("https://evil.example").expect("origin")),
                DownloadDispositionV1::Attachment,
                timestamp(63),
            ),
        )
        .await
        .expect_err("disallowed CORS origin");
    assert_eq!(disallowed.kind(), StorageFailureKind::NotFound);
}

#[tokio::test]
async fn altered_expired_wrong_operation_and_cross_tenant_grants_are_opaque() {
    let tenant = TenantId::new();
    let other_tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let key = source_key(tenant, video, 1);
    let bytes = b"0123456789abcdefghijXYZ";
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = DeterministicMultipartJournal::default();
    let service = build_service(&provider, &journal);
    let create_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth,
                idem("create-abuse-0001"),
                spec(key.clone(), bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create");
    let put_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;

    journal
        .inject_failure(
            MultipartJournalOperationV1::GetGrant,
            StorageFailure::new(StorageFailureKind::Timeout),
        )
        .expect("inject grant lookup failure");
    let other_key = source_key(other_tenant, VideoId::new(), 1);
    assert_eq!(
        service
            .put_part(
                StorageRequestContext::new(other_tenant, CorrelationId::new()),
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), other_key, 20),
                    idem("cross-grant-fault-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(&bytes[..10]),
                    bytes[..10].to_vec(),
                ),
            )
            .await
            .expect_err("cross-tenant grant is hidden before journal fault")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                    idem("authorized-grant-fault-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(&bytes[..10]),
                    bytes[..10].to_vec(),
                ),
            )
            .await
            .expect_err("authorized lookup consumes journal fault")
            .kind(),
        StorageFailureKind::Timeout
    );

    provider
        .inject_failure(
            MultipartProviderOperationV1::PutPart,
            StorageFailure::new(StorageFailureKind::Timeout),
        )
        .expect("inject timeout");
    let altered = MultipartAuthorizationV1::new(
        put_auth.grant_id(),
        MultipartGrantSecret::parse("multipart-test-secret-material-9999").expect("secret"),
    );
    let wrong_secret = service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(altered, plan.upload_id(), key.clone(), 20),
                idem("abuse-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(&bytes[..10]),
                bytes[..10].to_vec(),
            ),
        )
        .await
        .expect_err("altered secret");
    assert_eq!(wrong_secret.kind(), StorageFailureKind::NotFound);
    let injected = service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                idem("valid-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(&bytes[..10]),
                bytes[..10].to_vec(),
            ),
        )
        .await
        .expect_err("authorized call consumes timeout");
    assert_eq!(injected.kind(), StorageFailureKind::Timeout);

    let wrong_operation_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::ListParts,
    )
    .await;
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(wrong_operation_auth, plan.upload_id(), key.clone(), 20,),
                    idem("wrong-operation-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(&bytes[..10]),
                    bytes[..10].to_vec(),
                ),
            )
            .await
            .expect_err("wrong operation")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), key.clone(), 1_000),
                    idem("expired-grant-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(&bytes[..10]),
                    bytes[..10].to_vec(),
                ),
            )
            .await
            .expect_err("expired")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        service
            .put_part(
                StorageRequestContext::new(other_tenant, CorrelationId::new()),
                PutMultipartPartCommandV1::new(
                    scoped(put_auth, plan.upload_id(), key, 20),
                    idem("cross-tenant-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(&bytes[..10]),
                    bytes[..10].to_vec(),
                ),
            )
            .await
            .expect_err("cross tenant")
            .kind(),
        StorageFailureKind::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_and_abort_are_linearizable_and_replays_are_terminal_safe() {
    use std::sync::Arc;

    let tenant = TenantId::new();
    let context = context(tenant);
    let key = source_key(tenant, VideoId::new(), 1);
    let bytes = b"0123456789";
    let provider = Arc::new(DeterministicMultipartObjectStore::new(
        capabilities(),
        probe(),
    ));
    let journal = Arc::new(DeterministicMultipartJournal::default());
    let service = build_service(provider.as_ref(), journal.as_ref());
    let create_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth,
                idem("create-race-0001"),
                spec(key.clone(), bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create");
    let put_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;
    service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth, plan.upload_id(), key.clone(), 20),
                idem("race-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(bytes),
                bytes.to_vec(),
            ),
        )
        .await
        .expect("part");
    let complete_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Complete,
    )
    .await;
    let abort_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Abort,
    )
    .await;
    let upload_id = plan.upload_id();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let complete_task = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        let barrier = Arc::clone(&barrier);
        let key = key.clone();
        tokio::spawn(async move {
            let service = build_service(provider.as_ref(), journal.as_ref());
            barrier.wait().await;
            service
                .complete(
                    context,
                    IdempotentMultipartCommandV1::new(
                        scoped(complete_auth, upload_id, key, 30),
                        idem("race-complete-0001"),
                    ),
                )
                .await
        })
    };
    let abort_task = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        let barrier = Arc::clone(&barrier);
        let key = key.clone();
        tokio::spawn(async move {
            let service = build_service(provider.as_ref(), journal.as_ref());
            barrier.wait().await;
            service
                .abort(
                    context,
                    IdempotentMultipartCommandV1::new(
                        scoped(abort_auth, upload_id, key, 30),
                        idem("race-abort-0001"),
                    ),
                )
                .await
        })
    };
    barrier.wait().await;
    let completed = complete_task.await.expect("join complete");
    let aborted = abort_task.await.expect("join abort");
    assert!(
        completed.is_ok()
            ^ aborted.as_ref().is_ok_and(|receipt| {
                matches!(
                    receipt.disposition(),
                    frame_ports::ProviderAbortDispositionV1::Aborted
                        | frame_ports::ProviderAbortDispositionV1::AlreadyAborted
                )
            })
    );
    let snapshot = journal
        .get_upload(context, upload_id)
        .await
        .expect("journal")
        .expect("snapshot");
    assert!(matches!(
        snapshot.phase(),
        MultipartJournalPhaseV1::ProviderCompleted | MultipartJournalPhaseV1::Aborted
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_first_finalize_calls_share_the_first_durable_timestamp() {
    let tenant = TenantId::new();
    let base_context = context(tenant);
    let video = VideoId::new();
    let key = source_key(tenant, video, 1);
    let wrong_key = source_key(tenant, video, 99);
    let bytes = b"0123456789";
    let provider = Arc::new(CorruptingMultipartProvider::new(wrong_key.clone()));
    let journal = Arc::new(CorruptingMultipartJournal::new(wrong_key));
    let service = build_service(provider.as_ref(), journal.as_ref());
    let upload_id =
        prepare_provider_completed_upload(&service, base_context, key.clone(), bytes).await;
    assert_eq!(provider.complete_calls(), 1);
    let finalize_auth = grant(
        &service,
        base_context,
        key.clone(),
        Some(upload_id),
        MultipartOperationV1::Finalize,
    )
    .await;

    let context_one = StorageRequestContext::new(tenant, CorrelationId::new());
    let context_two = StorageRequestContext::new(tenant, CorrelationId::new());
    let correlation_one = context_one.correlation_id();
    let correlation_two = context_two.correlation_id();
    assert_ne!(correlation_one, correlation_two);
    journal.set_finalize_barrier(Some(Arc::new(tokio::sync::Barrier::new(2))));
    let first = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        let authorization = finalize_auth.clone();
        let key = key.clone();
        tokio::spawn(async move {
            build_service(provider.as_ref(), journal.as_ref())
                .finalize(
                    context_one,
                    IdempotentMultipartCommandV1::new(
                        scoped(authorization, upload_id, key, 50),
                        idem("concurrent-first-finalize"),
                    ),
                )
                .await
        })
    };
    let second = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        let authorization = finalize_auth.clone();
        let key = key.clone();
        tokio::spawn(async move {
            build_service(provider.as_ref(), journal.as_ref())
                .finalize(
                    context_two,
                    IdempotentMultipartCommandV1::new(
                        scoped(authorization, upload_id, key, 60),
                        idem("concurrent-first-finalize"),
                    ),
                )
                .await
        })
    };
    let first = first
        .await
        .expect("join first finalize")
        .expect("first finalize");
    let second = second
        .await
        .expect("join second finalize")
        .expect("second finalize");
    journal.set_finalize_barrier(None);

    assert_same_durable_finalize(&first, &second);
    assert_eq!(first.correlation_id(), correlation_one);
    assert_eq!(second.correlation_id(), correlation_two);
    assert!(matches!(first.finalized_at().get(), 50 | 60));
    assert_eq!(provider.complete_calls(), 1);
    let durable = journal
        .get_finalize(base_context, upload_id)
        .await
        .expect("durable finalize lookup")
        .expect("durable finalize");
    assert_same_durable_finalize(&durable, &first);

    let replay_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let replay = service
        .finalize(
            replay_context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth, upload_id, key, 70),
                idem("concurrent-first-finalize"),
            ),
        )
        .await
        .expect("unpoisoned client replay");
    assert_same_durable_finalize(&replay, &durable);
    assert_eq!(replay.correlation_id(), replay_context.correlation_id());
    assert_eq!(provider.complete_calls(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_reconciliation_finalize_calls_share_the_first_durable_timestamp() {
    let tenant = TenantId::new();
    let base_context = context(tenant);
    let video = VideoId::new();
    let key = source_key(tenant, video, 1);
    let wrong_key = source_key(tenant, video, 99);
    let bytes = b"0123456789";
    let provider = Arc::new(CorruptingMultipartProvider::new(wrong_key.clone()));
    let journal = Arc::new(CorruptingMultipartJournal::new(wrong_key));
    let service = build_service(provider.as_ref(), journal.as_ref());
    let upload_id =
        prepare_provider_completed_upload(&service, base_context, key.clone(), bytes).await;
    assert_eq!(provider.complete_calls(), 1);

    let context_one = StorageRequestContext::new(tenant, CorrelationId::new());
    let context_two = StorageRequestContext::new(tenant, CorrelationId::new());
    let correlation_one = context_one.correlation_id();
    let correlation_two = context_two.correlation_id();
    assert_ne!(correlation_one, correlation_two);
    journal.set_finalize_barrier(Some(Arc::new(tokio::sync::Barrier::new(2))));
    let first = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        tokio::spawn(async move {
            build_service(provider.as_ref(), journal.as_ref())
                .reconcile(context_one, upload_id, timestamp(50))
                .await
        })
    };
    let second = {
        let provider = Arc::clone(&provider);
        let journal = Arc::clone(&journal);
        tokio::spawn(async move {
            build_service(provider.as_ref(), journal.as_ref())
                .reconcile(context_two, upload_id, timestamp(60))
                .await
        })
    };
    let MultipartReconcileOutcomeV1::Finalized(first) = first
        .await
        .expect("join first reconciliation")
        .expect("first reconciliation")
    else {
        panic!("first reconciliation must finalize");
    };
    let MultipartReconcileOutcomeV1::Finalized(second) = second
        .await
        .expect("join second reconciliation")
        .expect("second reconciliation")
    else {
        panic!("second reconciliation must finalize");
    };
    journal.set_finalize_barrier(None);

    assert_same_durable_finalize(&first, &second);
    assert_eq!(first.correlation_id(), correlation_one);
    assert_eq!(second.correlation_id(), correlation_two);
    assert!(matches!(first.finalized_at().get(), 50 | 60));
    assert_eq!(provider.complete_calls(), 1);
    let durable = journal
        .get_finalize(base_context, upload_id)
        .await
        .expect("durable finalize lookup")
        .expect("durable finalize");
    assert_same_durable_finalize(&durable, &first);

    let finalize_auth = grant(
        &service,
        base_context,
        key.clone(),
        Some(upload_id),
        MultipartOperationV1::Finalize,
    )
    .await;
    let client_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let client_replay = service
        .finalize(
            client_context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth, upload_id, key, 70),
                idem("client-after-reconcile-race"),
            ),
        )
        .await
        .expect("client namespace remains unpoisoned");
    assert_same_durable_finalize(&client_replay, &durable);
    assert_eq!(
        client_replay.correlation_id(),
        client_context.correlation_id()
    );
    assert_eq!(provider.complete_calls(), 1);
}

#[tokio::test]
async fn grant_rotation_overlap_revocation_and_key_retirement_are_fail_closed() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = DeterministicMultipartJournal::default();
    let old_service = build_service_with_key_versions(&provider, &journal, 1, &[1]);
    let overlap_service = build_service_with_key_versions(&provider, &journal, 2, &[1, 2]);
    let retired_service = build_service_with_key_versions(&provider, &journal, 2, &[2]);
    let bytes = b"0123456789";

    let first_key = source_key(tenant, video, 1);
    let first_secret = secret();
    let first_id = MultipartGrantId::new();
    old_service
        .register_grant(
            context,
            first_id,
            &first_secret,
            MultipartGrantKeyVersion::new(1).expect("version"),
            MultipartGrantScopeV1::new(
                tenant,
                first_key.clone(),
                None,
                MultipartOperationV1::Create,
            )
            .expect("scope"),
            timestamp(1),
            timestamp(1_000),
        )
        .await
        .expect("old grant");
    let first_auth = MultipartAuthorizationV1::new(first_id, first_secret);
    let plan = overlap_service
        .create(
            context,
            CreateMultipartCommandV1::new(
                first_auth.clone(),
                idem("rotation-overlap-0001"),
                spec(first_key.clone(), bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("old grant accepted during overlap");
    assert_eq!(plan.key(), &first_key);
    overlap_service
        .revoke_grant(context, first_id, timestamp(20))
        .await
        .expect("revoke");
    assert_eq!(
        overlap_service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    first_auth,
                    idem("rotation-overlap-0001"),
                    spec(first_key, bytes),
                    timestamp(900),
                    timestamp(21),
                ),
            )
            .await
            .expect_err("revoked grant")
            .kind(),
        StorageFailureKind::NotFound
    );

    let second_key = source_key(tenant, video, 2);
    let second_secret = secret();
    let second_id = MultipartGrantId::new();
    old_service
        .register_grant(
            context,
            second_id,
            &second_secret,
            MultipartGrantKeyVersion::new(1).expect("version"),
            MultipartGrantScopeV1::new(
                tenant,
                second_key.clone(),
                None,
                MultipartOperationV1::Create,
            )
            .expect("scope"),
            timestamp(1),
            timestamp(1_000),
        )
        .await
        .expect("second old grant");
    assert_eq!(
        retired_service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    MultipartAuthorizationV1::new(second_id, second_secret),
                    idem("retired-key-0001"),
                    spec(second_key.clone(), bytes),
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("retired verification key")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        overlap_service
            .register_grant(
                context,
                MultipartGrantId::new(),
                &secret(),
                MultipartGrantKeyVersion::new(1).expect("version"),
                MultipartGrantScopeV1::new(tenant, second_key, None, MultipartOperationV1::Create,)
                    .expect("scope"),
                timestamp(1),
                timestamp(1_000),
            )
            .await
            .expect_err("new grants must use active key")
            .kind(),
        StorageFailureKind::NotFound
    );
}

#[tokio::test]
async fn create_response_activation_crashes_recover_or_expire_without_duplicate_sessions() {
    let tenant = TenantId::new();
    let context = context(tenant);
    let bytes = b"0123456789";
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = DeterministicMultipartJournal::default();
    let service = build_service(&provider, &journal);
    let key = source_key(tenant, VideoId::new(), 1);
    let create_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    journal
        .inject_failure(
            MultipartJournalOperationV1::Activate,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("activate failure");
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    create_auth,
                    idem("crash-create-recover"),
                    spec(key.clone(), bytes),
                    timestamp(100),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("provider response preceded activation crash")
            .kind(),
        StorageFailureKind::Unavailable
    );
    assert_eq!(provider.active_upload_count().expect("active uploads"), 1);
    let candidate = service
        .reconciliation_candidates(context, 10)
        .await
        .expect("candidate")
        .into_iter()
        .find(|candidate| candidate.spec().key() == &key)
        .expect("creating candidate");
    provider
        .inject_failure(
            MultipartProviderOperationV1::Lookup,
            StorageFailure::new(StorageFailureKind::Timeout),
        )
        .expect("lookup failure");
    provider
        .inject_failure(
            MultipartProviderOperationV1::Create,
            StorageFailure::new(StorageFailureKind::Timeout),
        )
        .expect("create must remain unused");
    assert_eq!(
        service
            .reconcile(context, candidate.upload_id(), timestamp(20))
            .await
            .expect_err("lookup unavailability never falls through to create")
            .kind(),
        StorageFailureKind::Timeout
    );
    assert_eq!(provider.active_upload_count().expect("active uploads"), 1);
    let MultipartReconcileOutcomeV1::Activated(recovered) = service
        .reconcile(context, candidate.upload_id(), timestamp(20))
        .await
        .expect("lookup recovers the existing provider session")
    else {
        panic!("expected activation");
    };
    assert_eq!(recovered.upload_id(), candidate.upload_id());
    assert_eq!(provider.active_upload_count().expect("active uploads"), 1);

    let unused_create_key = source_key(tenant, VideoId::new(), 2);
    let unused_create_auth = grant(
        &service,
        context,
        unused_create_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    unused_create_auth,
                    idem("lookup-did-not-create"),
                    spec(unused_create_key, bytes),
                    timestamp(100),
                    timestamp(20),
                ),
            )
            .await
            .expect_err("queued create failure proves lookup did not mint a second session")
            .kind(),
        StorageFailureKind::Timeout
    );
    assert_eq!(provider.active_upload_count().expect("active uploads"), 1);
    let abort_auth = grant(
        &service,
        context,
        key.clone(),
        Some(candidate.upload_id()),
        MultipartOperationV1::Abort,
    )
    .await;
    service
        .abort(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(abort_auth, candidate.upload_id(), key, 30),
                idem("recovered-create-abort"),
            ),
        )
        .await
        .expect("cleanup recovered upload");
    assert_eq!(provider.active_upload_count().expect("active uploads"), 0);

    let expiry_provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let expiry_journal = DeterministicMultipartJournal::default();
    let expiry_service = build_service(&expiry_provider, &expiry_journal);
    let expiry_key = source_key(tenant, VideoId::new(), 3);
    let expiry_auth = grant(
        &expiry_service,
        context,
        expiry_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    expiry_journal
        .inject_failure(
            MultipartJournalOperationV1::Activate,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("activate failure");
    assert_eq!(
        expiry_service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    expiry_auth,
                    idem("crash-create-expiry"),
                    spec(expiry_key.clone(), bytes),
                    timestamp(100),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("activation crash")
            .kind(),
        StorageFailureKind::Unavailable
    );
    let expiry_candidate = expiry_service
        .reconciliation_candidates(context, 10)
        .await
        .expect("candidate")
        .into_iter()
        .find(|candidate| candidate.spec().key() == &expiry_key)
        .expect("creating candidate");
    let MultipartReconcileOutcomeV1::Aborted(receipt) = expiry_service
        .reconcile(context, expiry_candidate.upload_id(), timestamp(100))
        .await
        .expect("expired recovered session is aborted")
    else {
        panic!("expected abort");
    };
    assert!(matches!(
        receipt.disposition(),
        frame_ports::ProviderAbortDispositionV1::Aborted
            | frame_ports::ProviderAbortDispositionV1::AlreadyAborted
    ));
    assert_eq!(
        expiry_provider
            .active_upload_count()
            .expect("active uploads"),
        0
    );
}

#[tokio::test]
async fn stale_creating_and_uploading_sessions_are_reconciled_to_abort() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = DeterministicMultipartJournal::default();
    let service = build_service(&provider, &journal);
    let bytes = b"0123456789";

    let creating_key = source_key(tenant, video, 1);
    let creating_auth = grant(
        &service,
        context,
        creating_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    provider
        .inject_failure(
            MultipartProviderOperationV1::Create,
            StorageFailure::new(StorageFailureKind::Timeout),
        )
        .expect("create failure");
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    creating_auth,
                    idem("stale-creating-0001"),
                    spec(creating_key, bytes),
                    timestamp(100),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("provider create timeout")
            .kind(),
        StorageFailureKind::Timeout
    );
    let candidates = service
        .reconciliation_candidates(context, 10)
        .await
        .expect("candidates");
    let creating = candidates
        .iter()
        .find(|candidate| candidate.phase() == MultipartJournalPhaseV1::Creating)
        .expect("creating candidate");
    let MultipartReconcileOutcomeV1::Aborted(receipt) = service
        .reconcile(context, creating.upload_id(), timestamp(100))
        .await
        .expect("creating cleanup")
    else {
        panic!("expected creating abort");
    };
    assert_eq!(
        receipt.disposition(),
        frame_ports::ProviderAbortDispositionV1::AlreadyAborted
    );

    let uploading_key = source_key(tenant, video, 2);
    let uploading_auth = grant(
        &service,
        context,
        uploading_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let uploading = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                uploading_auth,
                idem("stale-uploading-0001"),
                spec(uploading_key, bytes),
                timestamp(100),
                timestamp(10),
            ),
        )
        .await
        .expect("uploading session");
    let MultipartReconcileOutcomeV1::Aborted(receipt) = service
        .reconcile(context, uploading.upload_id(), timestamp(100))
        .await
        .expect("uploading cleanup")
    else {
        panic!("expected uploading abort");
    };
    assert_eq!(
        receipt.disposition(),
        frame_ports::ProviderAbortDispositionV1::Aborted
    );
    assert_eq!(
        service
            .reconcile(context, uploading.upload_id(), timestamp(101))
            .await
            .expect("terminal replay"),
        MultipartReconcileOutcomeV1::AlreadyTerminal(MultipartJournalPhaseV1::Aborted)
    );
}

#[tokio::test]
async fn provider_part_reference_is_checked_before_capability_and_body_hashing() {
    let tenant = TenantId::new();
    let context = context(tenant);
    let key = source_key(tenant, VideoId::new(), 1);
    let bytes = b"0123456789";
    let provider = DeterministicMultipartObjectStore::new(
        capabilities().without(MultipartProviderOperationV1::PutPart),
        probe(),
    );
    let upload_id = MultipartUploadId::new();
    let session = provider
        .create_multipart(
            context,
            ProviderCreateMultipartRequestV1::new(
                upload_id,
                spec(key.clone(), bytes),
                timestamp(100),
                context.correlation_id(),
            ),
        )
        .await
        .expect("provider create");
    let bad_checksum = ChecksumSha256::parse("b".repeat(64)).expect("checksum");
    let wrong_reference = ProviderUploadReferenceV1::new(
        upload_id,
        key.clone(),
        ProviderMultipartHandleV1::parse("wrong-provider-handle").expect("handle"),
        context.correlation_id(),
    );
    assert_eq!(
        provider
            .put_part(
                context,
                ProviderPutPartRequestV1::new(
                    wrong_reference,
                    MultipartPartNumberV1::new(1).expect("part"),
                    bad_checksum.clone(),
                    bytes.to_vec(),
                ),
            )
            .await
            .expect_err("wrong handle is hidden before capability or hashing")
            .kind(),
        StorageFailureKind::NotFound
    );
    let correct_reference = ProviderUploadReferenceV1::new(
        upload_id,
        key,
        session.handle().clone(),
        context.correlation_id(),
    );
    assert_eq!(
        provider
            .put_part(
                context,
                ProviderPutPartRequestV1::new(
                    correct_reference,
                    MultipartPartNumberV1::new(1).expect("part"),
                    bad_checksum,
                    bytes.to_vec(),
                ),
            )
            .await
            .expect_err("capability is checked before hashing an authorized body")
            .kind(),
        StorageFailureKind::UnsupportedCapability
    );
}

#[tokio::test]
async fn missing_provider_operations_and_checksum_capability_fail_before_upload_state() {
    let tenant = TenantId::new();
    let context = context(tenant);
    let bytes = b"0123456789";

    for (provider_capabilities, suffix) in [
        (
            MultipartProviderCapabilitiesV1::full(
                size(5),
                size(10),
                10,
                size(100),
                size(100),
                false,
            )
            .expect("capabilities without checksum"),
            "checksum",
        ),
        (
            capabilities().without(MultipartProviderOperationV1::Complete),
            "complete",
        ),
        (
            capabilities().without(MultipartProviderOperationV1::Lookup),
            "lookup",
        ),
    ] {
        let provider = DeterministicMultipartObjectStore::new(provider_capabilities, probe());
        let journal = DeterministicMultipartJournal::default();
        let service = build_service(&provider, &journal);
        let key = source_key(tenant, VideoId::new(), 1);
        let authorization = grant(
            &service,
            context,
            key.clone(),
            None,
            MultipartOperationV1::Create,
        )
        .await;
        assert_eq!(
            service
                .create(
                    context,
                    CreateMultipartCommandV1::new(
                        authorization,
                        idem(&format!("missing-provider-{suffix}")),
                        spec(key, bytes),
                        timestamp(900),
                        timestamp(10),
                    ),
                )
                .await
                .expect_err("unsupported provider contract")
                .kind(),
            StorageFailureKind::UnsupportedCapability
        );
        assert!(
            service
                .reconciliation_candidates(context, 10)
                .await
                .expect("no upload state")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn hostile_adapter_successes_are_rejected_and_safe_retries_recover() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let key = source_key(tenant, video, 1);
    let wrong_key = source_key(tenant, video, 99);
    let bytes = b"0123456789";
    let provider = CorruptingMultipartProvider::new(wrong_key);
    let journal = DeterministicMultipartJournal::default();
    let service = build_service(&provider, &journal);

    let create_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    provider.attack(ATTACK_CREATE);
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    create_auth.clone(),
                    idem("hostile-create-0001"),
                    spec(key.clone(), bytes),
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("swapped create key")
            .kind(),
        StorageFailureKind::Integrity
    );
    provider.attack(ATTACK_NONE);
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth,
                idem("hostile-create-0001"),
                spec(key.clone(), bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("create recovery");

    let put_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;
    provider.attack(ATTACK_PART);
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(put_auth.clone(), plan.upload_id(), key.clone(), 20),
                    idem("hostile-part-0001"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(bytes),
                    bytes.to_vec(),
                ),
            )
            .await
            .expect_err("swapped part checksum")
            .kind(),
        StorageFailureKind::Integrity
    );
    provider.attack(ATTACK_NONE);
    service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth, plan.upload_id(), key.clone(), 20),
                idem("hostile-part-0001"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(bytes),
                bytes.to_vec(),
            ),
        )
        .await
        .expect("part recovery");

    let list_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::ListParts,
    )
    .await;
    provider.attack(ATTACK_LIST);
    assert_eq!(
        service
            .list_parts(
                context,
                scoped(list_auth, plan.upload_id(), key.clone(), 21),
            )
            .await
            .expect_err("swapped list key")
            .kind(),
        StorageFailureKind::Integrity
    );

    let complete_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Complete,
    )
    .await;
    provider.attack(ATTACK_COMPLETE);
    assert_eq!(
        service
            .complete(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(complete_auth.clone(), plan.upload_id(), key.clone(), 30),
                    idem("hostile-complete-0001"),
                ),
            )
            .await
            .expect_err("swapped completed key")
            .kind(),
        StorageFailureKind::Integrity
    );
    provider.attack(ATTACK_NONE);
    service
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth, plan.upload_id(), key.clone(), 30),
                idem("hostile-complete-0001"),
            ),
        )
        .await
        .expect("complete recovery");
    let get_auth = grant(
        &service,
        context,
        key.clone(),
        None,
        MultipartOperationV1::Get,
    )
    .await;
    provider
        .inner
        .inject_failure(
            MultipartProviderOperationV1::Get,
            StorageFailure::new(StorageFailureKind::Unavailable),
        )
        .expect("queued provider get failure");
    assert_eq!(
        service
            .download(
                context,
                PrivateDownloadCommandV1::new(
                    get_auth.clone(),
                    key.clone(),
                    PrivateDownloadMethodV1::Get,
                    None,
                    DownloadValidatorV1::None,
                    None,
                    DownloadDispositionV1::Inline,
                    timestamp(34),
                ),
            )
            .await
            .expect_err("unfinalized objects are not downloadable")
            .kind(),
        StorageFailureKind::NotFound
    );
    let finalize_auth = grant(
        &service,
        context,
        key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Finalize,
    )
    .await;
    service
        .finalize(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth, plan.upload_id(), key.clone(), 35),
                idem("hostile-finalize-0001"),
            ),
        )
        .await
        .expect("finalize before download");
    assert_eq!(
        service
            .download(
                context,
                PrivateDownloadCommandV1::new(
                    get_auth.clone(),
                    key.clone(),
                    PrivateDownloadMethodV1::Get,
                    None,
                    DownloadValidatorV1::None,
                    None,
                    DownloadDispositionV1::Inline,
                    timestamp(40),
                ),
            )
            .await
            .expect_err("unfinalized rejection did not consume provider I/O")
            .kind(),
        StorageFailureKind::Unavailable
    );
    for attack in [
        ATTACK_DOWNLOAD,
        ATTACK_DOWNLOAD_SIZE,
        ATTACK_DOWNLOAD_CHECKSUM,
        ATTACK_DOWNLOAD_CONTENT_TYPE,
        ATTACK_DOWNLOAD_VERSION,
        ATTACK_DOWNLOAD_ETAG,
        ATTACK_DOWNLOAD_LAST_MODIFIED,
    ] {
        provider.attack(attack);
        assert_eq!(
            service
                .download(
                    context,
                    PrivateDownloadCommandV1::new(
                        get_auth.clone(),
                        key.clone(),
                        PrivateDownloadMethodV1::Get,
                        None,
                        DownloadValidatorV1::None,
                        None,
                        DownloadDispositionV1::Inline,
                        timestamp(40),
                    ),
                )
                .await
                .expect_err("provider metadata must match durable finalization")
                .kind(),
            StorageFailureKind::Integrity
        );
    }
    for (attack, expected_kind) in [
        (ATTACK_BODY_EMPTY, StorageFailureKind::Integrity),
        (ATTACK_BODY_OVERSIZED, StorageFailureKind::Integrity),
        (ATTACK_BODY_EARLY_EOF, StorageFailureKind::Integrity),
        (ATTACK_BODY_EXTRA, StorageFailureKind::Integrity),
        (ATTACK_BODY_MIDSTREAM, StorageFailureKind::Unavailable),
        (ATTACK_BODY_CORRUPT, StorageFailureKind::Integrity),
    ] {
        provider.attack(attack);
        let mut response = service
            .download(
                context,
                PrivateDownloadCommandV1::new(
                    get_auth.clone(),
                    key.clone(),
                    PrivateDownloadMethodV1::Get,
                    None,
                    DownloadValidatorV1::None,
                    None,
                    DownloadDispositionV1::Inline,
                    timestamp(40),
                ),
            )
            .await
            .expect("metadata is validated before streamed bytes");
        let (consumed, error) = collect_until_body_error(&mut response).await;
        assert_eq!(error.kind(), expected_kind);
        if matches!(
            attack,
            ATTACK_BODY_EXTRA | ATTACK_BODY_MIDSTREAM | ATTACK_BODY_CORRUPT
        ) {
            assert!(!consumed.is_empty());
        } else {
            assert!(consumed.is_empty());
        }
    }
    provider.attack(ATTACK_BODY_TRACK_DROP);
    let before_drop = provider.body_drop_count();
    let response = service
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth.clone(),
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::None,
                None,
                DownloadDispositionV1::Inline,
                timestamp(40),
            ),
        )
        .await
        .expect("tracked download");
    drop(response);
    assert_eq!(provider.body_drop_count(), before_drop + 1);
    let mut cancelled = service
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth.clone(),
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::None,
                None,
                DownloadDispositionV1::Inline,
                timestamp(40),
            ),
        )
        .await
        .expect("cancellable download");
    let before_cancel = provider.body_drop_count();
    let body = cancelled.body_mut().expect("body");
    body.cancel();
    assert_eq!(provider.body_drop_count(), before_cancel + 1);
    assert!(body.next_chunk().await.expect("cancelled body").is_none());
    provider.attack(ATTACK_NONE);
    let mut recovered_download = service
        .download(
            context,
            PrivateDownloadCommandV1::new(
                get_auth,
                key.clone(),
                PrivateDownloadMethodV1::Get,
                None,
                DownloadValidatorV1::None,
                None,
                DownloadDispositionV1::Inline,
                timestamp(40),
            ),
        )
        .await
        .expect("download recovery");
    assert_eq!(collect_body(&mut recovered_download).await, bytes);

    let abort_key = source_key(tenant, video, 2);
    let abort_create_auth = grant(
        &service,
        context,
        abort_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let abort_plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                abort_create_auth,
                idem("hostile-abort-create"),
                spec(abort_key.clone(), bytes),
                timestamp(900),
                timestamp(50),
            ),
        )
        .await
        .expect("abort upload create");
    let abort_auth = grant(
        &service,
        context,
        abort_key.clone(),
        Some(abort_plan.upload_id()),
        MultipartOperationV1::Abort,
    )
    .await;
    provider.attack(ATTACK_ABORT);
    assert_eq!(
        service
            .abort(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        abort_auth.clone(),
                        abort_plan.upload_id(),
                        abort_key.clone(),
                        60,
                    ),
                    idem("hostile-abort-0001"),
                ),
            )
            .await
            .expect_err("swapped abort key")
            .kind(),
        StorageFailureKind::Integrity
    );
    provider.attack(ATTACK_NONE);
    let abort_recovery = service
        .abort(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(abort_auth, abort_plan.upload_id(), abort_key, 60),
                idem("hostile-abort-0001"),
            ),
        )
        .await
        .expect("abort recovery");
    assert_eq!(
        abort_recovery.disposition(),
        frame_ports::ProviderAbortDispositionV1::AlreadyAborted
    );
}

#[tokio::test]
async fn hostile_journal_successes_are_rejected_and_durable_state_recovers() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let wrong_key = source_key(tenant, video, 99);
    let provider = DeterministicMultipartObjectStore::new(capabilities(), probe());
    let journal = CorruptingMultipartJournal::new(wrong_key);
    let service = build_service(&provider, &journal);
    let bytes = b"0123456789";

    let claim_key = source_key(tenant, video, 1);
    let claim_secret = secret();
    let claim_grant_id = MultipartGrantId::new();
    let claim_scope = MultipartGrantScopeV1::new(
        tenant,
        claim_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .expect("scope");
    journal.attack(JOURNAL_ATTACK_REGISTER);
    assert_eq!(
        service
            .register_grant(
                context,
                claim_grant_id,
                &claim_secret,
                MultipartGrantKeyVersion::new(2).expect("version"),
                claim_scope.clone(),
                timestamp(1),
                timestamp(1_000),
            )
            .await
            .expect_err("mutated grant receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    service
        .register_grant(
            context,
            claim_grant_id,
            &claim_secret,
            MultipartGrantKeyVersion::new(2).expect("version"),
            claim_scope,
            timestamp(1),
            timestamp(1_000),
        )
        .await
        .expect("grant replay recovery");
    let claim_auth = MultipartAuthorizationV1::new(claim_grant_id, claim_secret);
    journal.attack(JOURNAL_ATTACK_GET_GRANT);
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    claim_auth.clone(),
                    idem("hostile-journal-claim"),
                    spec(claim_key.clone(), bytes),
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("journal returned a different grant id")
            .kind(),
        StorageFailureKind::NotFound
    );
    journal.attack(JOURNAL_ATTACK_CLAIM);
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    claim_auth.clone(),
                    idem("hostile-journal-claim"),
                    spec(claim_key.clone(), bytes),
                    timestamp(900),
                    timestamp(10),
                ),
            )
            .await
            .expect_err("structurally corrupt claim")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    service
        .create(
            context,
            CreateMultipartCommandV1::new(
                claim_auth,
                idem("hostile-journal-claim"),
                spec(claim_key, bytes),
                timestamp(900),
                timestamp(10),
            ),
        )
        .await
        .expect("claim recovery");

    let lifecycle_key = source_key(tenant, video, 2);
    let create_auth = grant(
        &service,
        context,
        lifecycle_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    journal.attack(JOURNAL_ATTACK_ACTIVATE);
    assert_eq!(
        service
            .create(
                context,
                CreateMultipartCommandV1::new(
                    create_auth.clone(),
                    idem("hostile-journal-activate"),
                    spec(lifecycle_key.clone(), bytes),
                    timestamp(900),
                    timestamp(20),
                ),
            )
            .await
            .expect_err("corrupt activation receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    let plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                create_auth,
                idem("hostile-journal-activate"),
                spec(lifecycle_key.clone(), bytes),
                timestamp(900),
                timestamp(20),
            ),
        )
        .await
        .expect("activation recovery");

    let put_auth = grant(
        &service,
        context,
        lifecycle_key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::PutPart,
    )
    .await;
    journal.attack(JOURNAL_ATTACK_PART);
    assert_eq!(
        service
            .put_part(
                context,
                PutMultipartPartCommandV1::new(
                    scoped(
                        put_auth.clone(),
                        plan.upload_id(),
                        lifecycle_key.clone(),
                        30
                    ),
                    idem("hostile-journal-part"),
                    MultipartPartNumberV1::new(1).expect("part"),
                    ChecksumSha256::digest_bytes(bytes),
                    bytes.to_vec(),
                ),
            )
            .await
            .expect_err("mutated part journal receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    service
        .put_part(
            context,
            PutMultipartPartCommandV1::new(
                scoped(put_auth, plan.upload_id(), lifecycle_key.clone(), 30),
                idem("hostile-journal-part"),
                MultipartPartNumberV1::new(1).expect("part"),
                ChecksumSha256::digest_bytes(bytes),
                bytes.to_vec(),
            ),
        )
        .await
        .expect("part journal recovery");

    let complete_auth = grant(
        &service,
        context,
        lifecycle_key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Complete,
    )
    .await;
    journal.attack(JOURNAL_ATTACK_COMPLETE);
    assert_eq!(
        service
            .complete(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        complete_auth.clone(),
                        plan.upload_id(),
                        lifecycle_key.clone(),
                        40,
                    ),
                    idem("hostile-journal-complete"),
                ),
            )
            .await
            .expect_err("mutated complete journal receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    service
        .complete(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(complete_auth, plan.upload_id(), lifecycle_key.clone(), 40),
                idem("hostile-journal-complete"),
            ),
        )
        .await
        .expect("complete journal recovery");

    let finalize_auth = grant(
        &service,
        context,
        lifecycle_key.clone(),
        Some(plan.upload_id()),
        MultipartOperationV1::Finalize,
    )
    .await;
    journal.attack(JOURNAL_ATTACK_FINALIZE_INPUT);
    assert_eq!(
        service
            .finalize(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        finalize_auth.clone(),
                        plan.upload_id(),
                        lifecycle_key.clone(),
                        50,
                    ),
                    idem("hostile-journal-finalize-input"),
                ),
            )
            .await
            .expect_err("journal rejects finalization predating provider metadata")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_FINALIZE);
    assert_eq!(
        service
            .finalize(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        finalize_auth.clone(),
                        plan.upload_id(),
                        lifecycle_key.clone(),
                        50,
                    ),
                    idem("hostile-journal-finalize"),
                ),
            )
            .await
            .expect_err("mutated finalize journal receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    let finalized = service
        .finalize(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(finalize_auth, plan.upload_id(), lifecycle_key, 51),
                idem("hostile-journal-finalize"),
            ),
        )
        .await
        .expect("finalize journal recovery");
    assert_eq!(finalized.finalized_at(), timestamp(50));

    let abort_key = source_key(tenant, video, 3);
    let abort_create_auth = grant(
        &service,
        context,
        abort_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let abort_plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                abort_create_auth,
                idem("hostile-journal-abort-create"),
                spec(abort_key.clone(), bytes),
                timestamp(900),
                timestamp(60),
            ),
        )
        .await
        .expect("abort create");
    let abort_auth = grant(
        &service,
        context,
        abort_key.clone(),
        Some(abort_plan.upload_id()),
        MultipartOperationV1::Abort,
    )
    .await;
    journal.attack(JOURNAL_ATTACK_ABORT);
    assert_eq!(
        service
            .abort(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        abort_auth.clone(),
                        abort_plan.upload_id(),
                        abort_key.clone(),
                        70,
                    ),
                    idem("hostile-journal-abort"),
                ),
            )
            .await
            .expect_err("mutated abort journal receipt")
            .kind(),
        StorageFailureKind::Integrity
    );
    journal.attack(JOURNAL_ATTACK_NONE);
    let aborted = service
        .abort(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(
                    abort_auth.clone(),
                    abort_plan.upload_id(),
                    abort_key.clone(),
                    70,
                ),
                idem("hostile-journal-abort"),
            ),
        )
        .await
        .expect("abort journal recovery");
    assert_eq!(
        aborted.disposition(),
        frame_ports::ProviderAbortDispositionV1::Aborted
    );
    let abort_retry_context = StorageRequestContext::new(tenant, CorrelationId::new());
    let rebound = service
        .abort(
            abort_retry_context,
            IdempotentMultipartCommandV1::new(
                scoped(
                    abort_auth.clone(),
                    abort_plan.upload_id(),
                    abort_key.clone(),
                    70,
                ),
                idem("hostile-journal-abort"),
            ),
        )
        .await
        .expect("abort replay with fresh correlation");
    assert_eq!(
        rebound.correlation_id(),
        abort_retry_context.correlation_id()
    );
    assert_eq!(
        service
            .abort(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(abort_auth, abort_plan.upload_id(), abort_key, 70),
                    idem("hostile-journal-abort-create"),
                ),
            )
            .await
            .expect_err("terminal abort still checks replay operation")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );

    let fingerprint_key = source_key(tenant, video, 4);
    let fingerprint_create_auth = grant(
        &service,
        context,
        fingerprint_key.clone(),
        None,
        MultipartOperationV1::Create,
    )
    .await;
    let fingerprint_plan = service
        .create(
            context,
            CreateMultipartCommandV1::new(
                fingerprint_create_auth,
                idem("terminal-fingerprint-create"),
                spec(fingerprint_key.clone(), bytes),
                timestamp(900),
                timestamp(80),
            ),
        )
        .await
        .expect("fingerprint upload");
    let fingerprint_abort_auth = grant(
        &service,
        context,
        fingerprint_key.clone(),
        Some(fingerprint_plan.upload_id()),
        MultipartOperationV1::Abort,
    )
    .await;
    service
        .abort(
            context,
            IdempotentMultipartCommandV1::new(
                scoped(
                    fingerprint_abort_auth.clone(),
                    fingerprint_plan.upload_id(),
                    fingerprint_key.clone(),
                    90,
                ),
                idem("terminal-fingerprint-own-key"),
            ),
        )
        .await
        .expect("terminal fingerprint upload abort");
    assert_eq!(
        service
            .abort(
                context,
                IdempotentMultipartCommandV1::new(
                    scoped(
                        fingerprint_abort_auth,
                        fingerprint_plan.upload_id(),
                        fingerprint_key,
                        90,
                    ),
                    idem("hostile-journal-abort"),
                ),
            )
            .await
            .expect_err("terminal abort checks the same-operation fingerprint")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
}

#[test]
fn all_secret_and_media_debug_surfaces_are_redacted() {
    let secret_text = "multipart-test-secret-material-0001";
    let material = MultipartGrantSecret::parse(secret_text).expect("secret");
    let authorization = MultipartAuthorizationV1::new(MultipartGrantId::new(), material);
    assert!(!format!("{authorization:?}").contains(secret_text));
    let key = MultipartGrantKeyMaterialV1::parse(vec![0x41; 32]).expect("key");
    assert_eq!(
        format!("{key:?}"),
        "MultipartGrantKeyMaterialV1([redacted])"
    );
    let tenant = TenantId::new();
    let media = b"TOP-SECRET-MEDIA-BYTES";
    let command = PutMultipartPartCommandV1::new(
        scoped(
            authorization,
            frame_domain::MultipartUploadId::new(),
            source_key(tenant, VideoId::new(), 1),
            10,
        ),
        idem("debug-part-0001"),
        MultipartPartNumberV1::new(1).expect("part"),
        ChecksumSha256::digest_bytes(media),
        media.to_vec(),
    );
    assert!(!format!("{command:?}").contains("TOP-SECRET"));
}
