use async_trait::async_trait;
use frame_domain::{
    AudioCodecV1, ByteSize, ChecksumSha256, ContentType, CorrelationId, IdempotencyKey,
    MediaContainerV1, MultipartGrantId, MultipartGrantRecordV1, MultipartPartNumberV1,
    MultipartUploadId, MultipartUploadSpecV1, ObjectRevision, ScopedObjectKey,
    StorageFileExtension, TenantId, TimestampMillis, TrustedMediaProbeV1, VideoCodecV1, VideoId,
    VideoObjectDescriptor,
};
use frame_ports::{
    JournalCreateOutcomeV1, JournalMutationOutcomeV1, MultipartJournalV1, MultipartObjectStoreV1,
    MultipartProviderCapabilitiesV1, MultipartReplayKeyV1, MultipartUploadSnapshotV1,
    ObjectByteRange, ProviderAbortReceiptV1, ProviderCompleteMultipartRequestV1,
    ProviderCompletedObjectV1, ProviderCreateMultipartRequestV1, ProviderDownloadBodyV1,
    ProviderDownloadMetadataV1, ProviderDownloadRequestV1, ProviderDownloadResponseV1,
    ProviderEntityTag, ProviderLookupMultipartRequestV1, ProviderMultipartHandleV1,
    ProviderMultipartSessionV1, ProviderObjectVersion, ProviderPartReceiptV1, ProviderPartsListV1,
    ProviderPutPartRequestV1, ProviderUploadReferenceV1, SourceFinalizeRecordV1, StorageFailure,
    StorageRequestContext,
};

struct ExternalProviderAdapter;

struct ExternalDownloadBody {
    next: Option<Vec<u8>>,
}

#[async_trait]
impl ProviderDownloadBodyV1 for ExternalDownloadBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure> {
        Ok(self.next.take())
    }
}

#[async_trait]
impl MultipartObjectStoreV1 for ExternalProviderAdapter {
    fn capabilities(&self) -> MultipartProviderCapabilitiesV1 {
        MultipartProviderCapabilitiesV1::full(size(1), size(10), 10, size(100), size(100), true)
            .expect("capabilities")
    }

    async fn create_multipart(
        &self,
        _context: StorageRequestContext,
        _request: ProviderCreateMultipartRequestV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn lookup_multipart(
        &self,
        _context: StorageRequestContext,
        _request: ProviderLookupMultipartRequestV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn list_parts(
        &self,
        _context: StorageRequestContext,
        _reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderPartsListV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn put_part(
        &self,
        _context: StorageRequestContext,
        _request: ProviderPutPartRequestV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn complete_multipart(
        &self,
        _context: StorageRequestContext,
        _request: ProviderCompleteMultipartRequestV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn abort_multipart(
        &self,
        _context: StorageRequestContext,
        _reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn head_private(
        &self,
        _context: StorageRequestContext,
        _request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn get_private(
        &self,
        _context: StorageRequestContext,
        _request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }
}

struct ExternalJournalAdapter;

#[async_trait]
impl MultipartJournalV1 for ExternalJournalAdapter {
    async fn register_grant(
        &self,
        _context: StorageRequestContext,
        _record: MultipartGrantRecordV1,
    ) -> Result<JournalMutationOutcomeV1<MultipartGrantRecordV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn get_grant(
        &self,
        _context: StorageRequestContext,
        _id: MultipartGrantId,
    ) -> Result<Option<MultipartGrantRecordV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn revoke_grant(
        &self,
        _context: StorageRequestContext,
        _id: MultipartGrantId,
        _revoked_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn claim_create(
        &self,
        _context: StorageRequestContext,
        _grant_id: MultipartGrantId,
        _now: TimestampMillis,
        _idempotency_key: IdempotencyKey,
        _fingerprint: ChecksumSha256,
        _draft: MultipartUploadSnapshotV1,
    ) -> Result<JournalCreateOutcomeV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn activate_upload(
        &self,
        _context: StorageRequestContext,
        _session: ProviderMultipartSessionV1,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn get_upload(
        &self,
        _context: StorageRequestContext,
        _upload_id: MultipartUploadId,
    ) -> Result<Option<MultipartUploadSnapshotV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn get_finalize(
        &self,
        _context: StorageRequestContext,
        _upload_id: MultipartUploadId,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn get_finalize_by_key(
        &self,
        _context: StorageRequestContext,
        _key: ScopedObjectKey,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn record_part(
        &self,
        _context: StorageRequestContext,
        _replay_key: MultipartReplayKeyV1,
        _fingerprint: ChecksumSha256,
        _receipt: ProviderPartReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderPartReceiptV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn record_provider_complete(
        &self,
        _context: StorageRequestContext,
        _replay_key: MultipartReplayKeyV1,
        _fingerprint: ChecksumSha256,
        _completed: ProviderCompletedObjectV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderCompletedObjectV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn finalize(
        &self,
        _context: StorageRequestContext,
        _replay_key: MultipartReplayKeyV1,
        _fingerprint: ChecksumSha256,
        _record: SourceFinalizeRecordV1,
    ) -> Result<JournalMutationOutcomeV1<SourceFinalizeRecordV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn abort(
        &self,
        _context: StorageRequestContext,
        _replay_key: MultipartReplayKeyV1,
        _fingerprint: ChecksumSha256,
        _receipt: ProviderAbortReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderAbortReceiptV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }

    async fn reconciliation_candidates(
        &self,
        _context: StorageRequestContext,
        _limit: u16,
    ) -> Result<Vec<MultipartUploadSnapshotV1>, StorageFailure> {
        Err(StorageFailure::unsupported())
    }
}

fn size(value: u64) -> ByteSize {
    ByteSize::new(value).expect("size")
}

fn key(tenant: TenantId) -> ScopedObjectKey {
    ScopedObjectKey::source(
        tenant,
        VideoId::new(),
        ObjectRevision::new(1).expect("revision"),
        VideoObjectDescriptor::Source {
            extension: StorageFileExtension::parse("webm").expect("extension"),
        },
    )
    .expect("key")
}

fn assert_provider<T: MultipartObjectStoreV1>() {}
fn assert_journal<T: MultipartJournalV1>() {}
fn assert_download_body<T: ProviderDownloadBodyV1>() {}

#[test]
fn external_provider_and_journal_traits_are_implementable() {
    assert_provider::<ExternalProviderAdapter>();
    assert_journal::<ExternalJournalAdapter>();
    assert_download_body::<ExternalDownloadBody>();
}

#[test]
fn external_adapters_can_construct_and_inspect_every_provider_binding() {
    let tenant = TenantId::new();
    let key = key(tenant);
    let upload_id = MultipartUploadId::new();
    let correlation = CorrelationId::new();
    let checksum = ChecksumSha256::digest_bytes(b"0123456789");
    let spec = MultipartUploadSpecV1::new(
        key.clone(),
        size(10),
        size(10),
        checksum.clone(),
        ContentType::parse("video/webm").expect("content type"),
        frame_domain::MultipartLimitsV1::new(
            size(1),
            size(10),
            10,
            size(100),
            size(10),
            frame_domain::DurationMillis::new(1_000).expect("ttl"),
        )
        .expect("limits"),
    )
    .expect("spec");
    let create = ProviderCreateMultipartRequestV1::new(
        upload_id,
        spec,
        TimestampMillis::new(1_000).expect("expiry"),
        correlation,
    );
    assert_eq!(create.upload_id(), upload_id);
    assert_eq!(create.key(), &key);
    assert_eq!(create.correlation_id(), correlation);
    let lookup = ProviderLookupMultipartRequestV1::new(upload_id, key.clone(), correlation);
    assert_eq!(lookup.upload_id(), upload_id);
    assert_eq!(lookup.key(), &key);
    assert_eq!(lookup.correlation_id(), correlation);
    let session = ProviderMultipartSessionV1::new(
        upload_id,
        key.clone(),
        ProviderMultipartHandleV1::parse("provider-upload-handle").expect("handle"),
        create.expires_at(),
        correlation,
    );
    let reference = ProviderUploadReferenceV1::new(
        upload_id,
        key.clone(),
        session.handle().clone(),
        correlation,
    );
    let part_number = MultipartPartNumberV1::new(1).expect("part");
    let request =
        ProviderPutPartRequestV1::new(reference, part_number, checksum, b"0123456789".to_vec());
    assert_eq!(request.reference().upload_id(), upload_id);
    assert_eq!(request.part_number(), part_number);
    assert_eq!(request.bytes(), b"0123456789");
    assert!(!format!("{request:?}").contains("0123456789"));

    let probe = TrustedMediaProbeV1::new(
        MediaContainerV1::Webm,
        VideoCodecV1::Vp9,
        AudioCodecV1::Opus,
        1280,
        720,
        1_000,
        30_000,
    )
    .expect("probe");
    assert_eq!(probe.width(), 1280);
}

#[tokio::test]
async fn external_download_body_is_boxable_pullable_and_debug_redacted() {
    let tenant = TenantId::new();
    let key = key(tenant);
    let correlation = CorrelationId::new();
    let bytes = b"0123456789".to_vec();
    let metadata = ProviderDownloadMetadataV1::new(
        key,
        size(10),
        ChecksumSha256::digest_bytes(&bytes),
        ContentType::parse("video/webm").expect("content type"),
        ProviderObjectVersion::parse("external-version").expect("version"),
        ProviderEntityTag::parse("external-etag").expect("etag"),
        TimestampMillis::new(10).expect("last modified"),
        correlation,
    );
    let mut response = ProviderDownloadResponseV1::Body {
        metadata,
        range: ObjectByteRange::new(0, 10).expect("range"),
        body: Box::new(ExternalDownloadBody {
            next: Some(bytes.clone()),
        }),
    };
    assert!(!format!("{response:?}").contains("0123456789"));
    let ProviderDownloadResponseV1::Body { body, .. } = &mut response else {
        panic!("expected body");
    };
    assert_eq!(body.next_chunk().await.expect("chunk"), Some(bytes));
    assert!(body.next_chunk().await.expect("eof").is_none());
}
