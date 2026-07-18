use frame_domain::ByteSize;
use frame_ports::{
    BeginUploadRequestV1, ObjectMetadataV1, ObjectStoreOperation, ObjectStoreV1,
    ObjectWriteReceiptV1, PutObjectRequestV1, StorageFailure, StorageFailureKind,
    StorageRequestContext, UploadBrokerV1, UploadDelivery, UploadMode, UploadPlanV1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImmutableWriteOutcome {
    Created(ObjectWriteReceiptV1),
    Reused(ObjectMetadataV1),
}

/// Capability-fenced orchestration for immutable objects and upload grants.
///
/// Provider adapters stay behind the ports. Every capability and size check happens before the
/// upload broker is invoked, so an unsupported storage mode cannot leave a remote upload behind.
pub struct ImmutableStorageService<'a> {
    store: &'a dyn ObjectStoreV1,
    broker: &'a dyn UploadBrokerV1,
}

impl<'a> ImmutableStorageService<'a> {
    #[must_use]
    pub const fn new(store: &'a dyn ObjectStoreV1, broker: &'a dyn UploadBrokerV1) -> Self {
        Self { store, broker }
    }

    pub async fn put_immutable(
        &self,
        context: StorageRequestContext,
        request: PutObjectRequestV1,
    ) -> Result<ImmutableWriteOutcome, StorageFailure> {
        if request.key().tenant_id() != context.tenant_id() {
            return Err(StorageFailure::new(StorageFailureKind::NotFound));
        }
        let capabilities = self.store.capabilities();
        for required in [
            ObjectStoreOperation::Put,
            ObjectStoreOperation::Head,
            ObjectStoreOperation::ConditionalCreate,
            ObjectStoreOperation::Sha256Integrity,
        ] {
            capabilities.require(required)?;
        }
        let expected_size = request_size(&request)?;
        if expected_size > capabilities.max_object_size() {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        let key = request.key().clone();
        let expected_content_type = request.content_type().clone();
        let expected_checksum = request.checksum_sha256().clone();
        let expected_cache_policy = request.cache_policy();
        let expected_tenant = context.tenant_id();
        let expected_correlation = context.correlation_id();
        match self.store.put(context, request).await {
            Ok(receipt) => {
                let metadata = receipt.metadata();
                if metadata.key() != &key
                    || metadata.key().tenant_id() != expected_tenant
                    || metadata.size() != expected_size
                    || metadata.content_type() != &expected_content_type
                    || metadata.checksum_sha256() != &expected_checksum
                    || metadata.cache_policy() != expected_cache_policy
                    || metadata.correlation_id() != expected_correlation
                {
                    return Err(StorageFailure::new(StorageFailureKind::Integrity));
                }
                Ok(ImmutableWriteOutcome::Created(receipt))
            }
            Err(error) if error.kind() == StorageFailureKind::PreconditionFailed => {
                let existing = self.store.head(context, &key).await?;
                if existing.key() != &key || existing.key().tenant_id() != expected_tenant {
                    return Err(StorageFailure::new(StorageFailureKind::Integrity));
                }
                if existing.size() == expected_size
                    && existing.content_type() == &expected_content_type
                    && existing.checksum_sha256() == &expected_checksum
                    && existing.cache_policy() == expected_cache_policy
                {
                    Ok(ImmutableWriteOutcome::Reused(existing))
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub async fn begin_upload(
        &self,
        context: StorageRequestContext,
        request: BeginUploadRequestV1,
    ) -> Result<UploadPlanV1, StorageFailure> {
        if request.key().tenant_id() != context.tenant_id() {
            return Err(StorageFailure::new(StorageFailureKind::NotFound));
        }
        let store_capabilities = self.store.capabilities();
        for required in [
            ObjectStoreOperation::Put,
            ObjectStoreOperation::Head,
            ObjectStoreOperation::ConditionalCreate,
            ObjectStoreOperation::Sha256Integrity,
        ] {
            store_capabilities.require(required)?;
        }
        if request.expected_size() > store_capabilities.max_object_size() {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        let broker_capabilities = self.broker.capabilities();
        if !broker_capabilities.supports(request.mode()) {
            return Err(StorageFailure::unsupported());
        }
        if !broker_capabilities.sha256_required() {
            return Err(StorageFailure::unsupported());
        }
        if request.expected_size() > broker_capabilities.max_object_size() {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        let expected_key = request.key().clone();
        let expected_mode = request.mode();
        let expected_size = request.expected_size();
        let expected_content_type = request.content_type().clone();
        let expected_checksum = request.checksum_sha256().clone();
        let expected_cache_policy = request.cache_policy();
        let expected_expiry = request.expires_at();
        let expected_tenant = context.tenant_id();
        let expected_correlation = context.correlation_id();
        let plan = self.broker.begin(context, request).await?;
        let delivery_matches = matches!(
            (plan.delivery(), expected_mode),
            (
                UploadDelivery::Brokered { .. },
                UploadMode::BrokeredSinglePut
            ) | (UploadDelivery::Direct { .. }, UploadMode::DirectSinglePut)
                | (
                    UploadDelivery::MultipartBrokered { .. },
                    UploadMode::Multipart
                )
        );
        if plan.key() != &expected_key
            || plan.key().tenant_id() != expected_tenant
            || plan.mode() != expected_mode
            || !delivery_matches
            || plan.expected_size() != expected_size
            || plan.content_type() != &expected_content_type
            || plan.checksum_sha256() != &expected_checksum
            || plan.cache_policy() != expected_cache_policy
            || plan.expires_at() != expected_expiry
            || plan.correlation_id() != expected_correlation
        {
            return Err(StorageFailure::new(StorageFailureKind::Integrity));
        }
        Ok(plan)
    }
}

fn request_size(request: &PutObjectRequestV1) -> Result<ByteSize, StorageFailure> {
    ByteSize::new(
        u64::try_from(request.bytes().len())
            .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?,
    )
    .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicU64, Ordering},
    };

    use frame_domain::{
        ChecksumSha256, ContentType, CorrelationId, ObjectRevision, StorageFileExtension, TenantId,
        TimestampMillis, VideoId, VideoObjectDescriptor,
    };
    use frame_ports::{
        BrokerUploadId, CompleteUploadRequestV1, CopyObjectRequestV1, DeleteObjectDisposition,
        DeleteObjectRequestV1, DeterministicObjectStore, DeterministicUploadBroker,
        DirectUploadAuthorization, ListObjectsPageV1, ListObjectsRequestV1, ObjectBodyV1,
        ObjectByteRange, ObjectCachePolicy, ObjectRangeBodyV1, ObjectStoreCapabilitiesV1,
        ProviderEntityTag, ProviderObjectVersion, SameOriginUploadPath, UploadBrokerCapabilitiesV1,
        UploadMode,
    };

    use super::*;

    fn key(tenant: TenantId, video: VideoId, revision: u64) -> frame_domain::ScopedObjectKey {
        frame_domain::ScopedObjectKey::source(
            tenant,
            video,
            ObjectRevision::new(revision).expect("revision"),
            VideoObjectDescriptor::Source {
                extension: StorageFileExtension::parse("webm").expect("extension"),
            },
        )
        .expect("key")
    }

    fn store_capabilities() -> ObjectStoreCapabilitiesV1 {
        ObjectStoreCapabilitiesV1::full(
            ByteSize::new(1_000_000).expect("size"),
            ByteSize::new(1_000_000).expect("size"),
            100,
        )
        .expect("capabilities")
    }

    fn broker_capabilities() -> UploadBrokerCapabilitiesV1 {
        UploadBrokerCapabilitiesV1::new(
            true,
            false,
            false,
            true,
            ByteSize::new(1_000_000).expect("size"),
        )
        .expect("capabilities")
    }

    fn context(tenant: TenantId) -> StorageRequestContext {
        StorageRequestContext::new(tenant, CorrelationId::new())
    }

    fn metadata(
        key: frame_domain::ScopedObjectKey,
        size: ByteSize,
        content_type: ContentType,
        checksum: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
        correlation_id: CorrelationId,
    ) -> ObjectMetadataV1 {
        ObjectMetadataV1::new(
            key,
            size,
            content_type,
            checksum,
            ProviderObjectVersion::parse("adversarial-version").expect("version"),
            ProviderEntityTag::parse("adversarial-etag").expect("etag"),
            cache_policy,
            TimestampMillis::new(1).expect("timestamp"),
            correlation_id,
        )
        .expect("metadata")
    }

    #[allow(clippy::too_many_arguments)]
    fn upload_plan(
        key: frame_domain::ScopedObjectKey,
        mode: UploadMode,
        size: ByteSize,
        content_type: ContentType,
        checksum: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
        expires_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> UploadPlanV1 {
        let delivery = match mode {
            UploadMode::BrokeredSinglePut => UploadDelivery::Brokered {
                path: SameOriginUploadPath::parse("/api/storage/uploads/adversarial")
                    .expect("path"),
            },
            UploadMode::DirectSinglePut => UploadDelivery::Direct {
                authorization: DirectUploadAuthorization::new(
                    "https://upload.invalid/adversarial",
                    BTreeMap::new(),
                )
                .expect("authorization"),
            },
            UploadMode::Multipart => UploadDelivery::MultipartBrokered {
                path: SameOriginUploadPath::parse("/api/storage/uploads/adversarial")
                    .expect("path"),
            },
        };
        UploadPlanV1::new(
            BrokerUploadId::parse("adversarial-upload").expect("id"),
            key,
            mode,
            delivery,
            size,
            content_type,
            checksum,
            cache_policy,
            expires_at,
            correlation_id,
        )
        .expect("plan")
    }

    struct AdversarialStore {
        capabilities: ObjectStoreCapabilitiesV1,
        capability_calls: AtomicU64,
        put_calls: AtomicU64,
        put_result: Result<ObjectWriteReceiptV1, StorageFailure>,
        head_result: Result<ObjectMetadataV1, StorageFailure>,
    }

    #[async_trait::async_trait]
    impl ObjectStoreV1 for AdversarialStore {
        fn capabilities(&self) -> ObjectStoreCapabilitiesV1 {
            self.capability_calls.fetch_add(1, Ordering::Relaxed);
            self.capabilities
        }

        async fn put(
            &self,
            _context: StorageRequestContext,
            _request: PutObjectRequestV1,
        ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
            self.put_calls.fetch_add(1, Ordering::Relaxed);
            self.put_result.clone()
        }

        async fn head(
            &self,
            _context: StorageRequestContext,
            _key: &frame_domain::ScopedObjectKey,
        ) -> Result<ObjectMetadataV1, StorageFailure> {
            self.head_result.clone()
        }

        async fn get(
            &self,
            _context: StorageRequestContext,
            _key: &frame_domain::ScopedObjectKey,
        ) -> Result<ObjectBodyV1, StorageFailure> {
            Err(StorageFailure::unsupported())
        }

        async fn get_range(
            &self,
            _context: StorageRequestContext,
            _key: &frame_domain::ScopedObjectKey,
            _range: ObjectByteRange,
        ) -> Result<ObjectRangeBodyV1, StorageFailure> {
            Err(StorageFailure::unsupported())
        }

        async fn copy(
            &self,
            _context: StorageRequestContext,
            _request: CopyObjectRequestV1,
        ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
            Err(StorageFailure::unsupported())
        }

        async fn delete(
            &self,
            _context: StorageRequestContext,
            _request: DeleteObjectRequestV1,
        ) -> Result<DeleteObjectDisposition, StorageFailure> {
            Err(StorageFailure::unsupported())
        }

        async fn list(
            &self,
            _context: StorageRequestContext,
            _request: ListObjectsRequestV1,
        ) -> Result<ListObjectsPageV1, StorageFailure> {
            Err(StorageFailure::unsupported())
        }
    }

    struct AdversarialBroker {
        plan: UploadPlanV1,
    }

    #[async_trait::async_trait]
    impl UploadBrokerV1 for AdversarialBroker {
        fn capabilities(&self) -> UploadBrokerCapabilitiesV1 {
            UploadBrokerCapabilitiesV1::new(
                true,
                true,
                true,
                true,
                ByteSize::new(1_000_000).expect("size"),
            )
            .expect("capabilities")
        }

        async fn begin(
            &self,
            _context: StorageRequestContext,
            _request: BeginUploadRequestV1,
        ) -> Result<UploadPlanV1, StorageFailure> {
            Ok(self.plan.clone())
        }

        async fn complete(
            &self,
            _context: StorageRequestContext,
            _request: CompleteUploadRequestV1,
        ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
            Err(StorageFailure::unsupported())
        }

        async fn abort(
            &self,
            _context: StorageRequestContext,
            _id: &BrokerUploadId,
        ) -> Result<(), StorageFailure> {
            Err(StorageFailure::unsupported())
        }
    }

    #[tokio::test]
    async fn identical_immutable_retry_reuses_and_changed_retry_never_overwrites() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let store = DeterministicObjectStore::new(store_capabilities());
        let broker = DeterministicUploadBroker::new(broker_capabilities());
        let service = ImmutableStorageService::new(&store, &broker);
        let bytes = b"immutable".to_vec();
        let checksum = ChecksumSha256::digest_bytes(&bytes);
        let request = || {
            PutObjectRequestV1::immutable(
                key(tenant, video, 1),
                bytes.clone(),
                ContentType::parse("video/webm").expect("content type"),
                checksum.clone(),
                ObjectCachePolicy::PrivateImmutable,
            )
            .expect("request")
        };
        assert!(matches!(
            service.put_immutable(context(tenant), request()).await,
            Ok(ImmutableWriteOutcome::Created(_))
        ));
        assert!(matches!(
            service.put_immutable(context(tenant), request()).await,
            Ok(ImmutableWriteOutcome::Reused(_))
        ));
        let changed = b"different".to_vec();
        let changed_request = PutObjectRequestV1::immutable(
            key(tenant, video, 1),
            changed.clone(),
            ContentType::parse("video/webm").expect("content type"),
            ChecksumSha256::digest_bytes(&changed),
            ObjectCachePolicy::PrivateImmutable,
        )
        .expect("request");
        assert_eq!(
            service
                .put_immutable(context(tenant), changed_request)
                .await
                .expect_err("collision"),
            StorageFailure::new(StorageFailureKind::PreconditionFailed)
        );
        assert_eq!(
            store
                .get(context(tenant), &key(tenant, video, 1))
                .await
                .expect("object")
                .bytes(),
            b"immutable"
        );
    }

    #[tokio::test]
    async fn cross_tenant_put_is_rejected_before_capabilities_size_or_adapter_calls() {
        let owner = TenantId::new();
        let attacker = TenantId::new();
        let video = VideoId::new();
        let bytes = b"0123456789";
        let request = || {
            PutObjectRequestV1::immutable(
                key(owner, video, 1),
                bytes.to_vec(),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(bytes),
                ObjectCachePolicy::PrivateImmutable,
            )
            .expect("request")
        };
        let broker = DeterministicUploadBroker::new(broker_capabilities());
        let stores = [
            AdversarialStore {
                capabilities: store_capabilities().without(ObjectStoreOperation::Put),
                capability_calls: AtomicU64::new(0),
                put_calls: AtomicU64::new(0),
                put_result: Err(StorageFailure::unsupported()),
                head_result: Err(StorageFailure::unsupported()),
            },
            AdversarialStore {
                capabilities: ObjectStoreCapabilitiesV1::full(
                    ByteSize::new(1).expect("size"),
                    ByteSize::new(1).expect("size"),
                    1,
                )
                .expect("capabilities"),
                capability_calls: AtomicU64::new(0),
                put_calls: AtomicU64::new(0),
                put_result: Err(StorageFailure::new(StorageFailureKind::QuotaExceeded)),
                head_result: Err(StorageFailure::unsupported()),
            },
        ];
        for store in &stores {
            assert_eq!(
                ImmutableStorageService::new(store, &broker)
                    .put_immutable(context(attacker), request())
                    .await
                    .expect_err("cross-tenant request"),
                StorageFailure::new(StorageFailureKind::NotFound)
            );
            assert_eq!(store.capability_calls.load(Ordering::Relaxed), 0);
            assert_eq!(store.put_calls.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn unsupported_mode_fails_before_broker_side_effect() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let store = DeterministicObjectStore::new(store_capabilities());
        let broker = DeterministicUploadBroker::new(broker_capabilities());
        let service = ImmutableStorageService::new(&store, &broker);
        let request = BeginUploadRequestV1::new(
            key(tenant, video, 1),
            UploadMode::DirectSinglePut,
            ByteSize::new(10).expect("size"),
            ContentType::parse("video/webm").expect("content type"),
            ChecksumSha256::digest_bytes(b"0123456789"),
            ObjectCachePolicy::PrivateImmutable,
            TimestampMillis::new(10_000).expect("timestamp"),
        )
        .expect("request");
        assert_eq!(
            service
                .begin_upload(context(tenant), request)
                .await
                .expect_err("unsupported"),
            StorageFailure::unsupported()
        );
        assert_eq!(broker.begin_call_count(), 0);
    }

    #[tokio::test]
    async fn missing_store_capability_fails_before_broker_side_effect() {
        let tenant = TenantId::new();
        let store = DeterministicObjectStore::new(
            store_capabilities().without(ObjectStoreOperation::ConditionalCreate),
        );
        let broker = DeterministicUploadBroker::new(broker_capabilities());
        let service = ImmutableStorageService::new(&store, &broker);
        let request = BeginUploadRequestV1::new(
            key(tenant, VideoId::new(), 1),
            UploadMode::BrokeredSinglePut,
            ByteSize::new(10).expect("size"),
            ContentType::parse("video/webm").expect("content type"),
            ChecksumSha256::digest_bytes(b"0123456789"),
            ObjectCachePolicy::PrivateImmutable,
            TimestampMillis::new(10_000).expect("timestamp"),
        )
        .expect("request");
        assert_eq!(
            service
                .begin_upload(context(tenant), request)
                .await
                .expect_err("unsupported"),
            StorageFailure::unsupported()
        );
        assert_eq!(broker.begin_call_count(), 0);
    }

    #[tokio::test]
    async fn broker_without_required_sha256_fails_before_broker_side_effect() {
        let tenant = TenantId::new();
        let store = DeterministicObjectStore::new(store_capabilities());
        let broker = DeterministicUploadBroker::new(
            UploadBrokerCapabilitiesV1::new(
                true,
                false,
                false,
                false,
                ByteSize::new(1_000_000).expect("size"),
            )
            .expect("capabilities"),
        );
        let service = ImmutableStorageService::new(&store, &broker);
        let request = BeginUploadRequestV1::new(
            key(tenant, VideoId::new(), 1),
            UploadMode::BrokeredSinglePut,
            ByteSize::new(10).expect("size"),
            ContentType::parse("video/webm").expect("content type"),
            ChecksumSha256::digest_bytes(b"0123456789"),
            ObjectCachePolicy::PrivateImmutable,
            TimestampMillis::new(10_000).expect("timestamp"),
        )
        .expect("request");
        assert_eq!(
            service
                .begin_upload(context(tenant), request)
                .await
                .expect_err("sha256 is mandatory"),
            StorageFailure::unsupported()
        );
        assert_eq!(broker.begin_call_count(), 0);
    }

    #[tokio::test]
    async fn put_and_head_success_postconditions_reject_adapter_drift() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let request_context = context(tenant);
        let expected_key = key(tenant, video, 1);
        let bytes = b"0123456789";
        let checksum = ChecksumSha256::digest_bytes(bytes);
        let request = || {
            PutObjectRequestV1::immutable(
                expected_key.clone(),
                bytes.to_vec(),
                ContentType::parse("video/webm").expect("content type"),
                checksum.clone(),
                ObjectCachePolicy::PrivateImmutable,
            )
            .expect("request")
        };
        let bad_receipt = ObjectWriteReceiptV1::new(metadata(
            key(tenant, video, 2),
            ByteSize::new(10).expect("size"),
            ContentType::parse("video/webm").expect("content type"),
            checksum.clone(),
            ObjectCachePolicy::PrivateImmutable,
            request_context.correlation_id(),
        ));
        let bad_store = AdversarialStore {
            capabilities: store_capabilities(),
            capability_calls: AtomicU64::new(0),
            put_calls: AtomicU64::new(0),
            put_result: Ok(bad_receipt),
            head_result: Err(StorageFailure::new(StorageFailureKind::NotFound)),
        };
        let broker = DeterministicUploadBroker::new(broker_capabilities());
        assert_eq!(
            ImmutableStorageService::new(&bad_store, &broker)
                .put_immutable(request_context, request())
                .await
                .expect_err("drifted put receipt"),
            StorageFailure::new(StorageFailureKind::Integrity)
        );

        let bad_head = metadata(
            key(tenant, video, 2),
            ByteSize::new(10).expect("size"),
            ContentType::parse("video/webm").expect("content type"),
            checksum.clone(),
            ObjectCachePolicy::PrivateImmutable,
            CorrelationId::new(),
        );
        let bad_store = AdversarialStore {
            capabilities: store_capabilities(),
            capability_calls: AtomicU64::new(0),
            put_calls: AtomicU64::new(0),
            put_result: Err(StorageFailure::new(StorageFailureKind::PreconditionFailed)),
            head_result: Ok(bad_head),
        };
        assert_eq!(
            ImmutableStorageService::new(&bad_store, &broker)
                .put_immutable(request_context, request())
                .await
                .expect_err("drifted head metadata"),
            StorageFailure::new(StorageFailureKind::Integrity)
        );
    }

    #[tokio::test]
    async fn upload_plan_postconditions_reject_every_bound_field_drift() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let request_context = context(tenant);
        let expected_key = key(tenant, video, 1);
        let expected_size = ByteSize::new(10).expect("size");
        let expected_type = ContentType::parse("video/webm").expect("content type");
        let expected_checksum = ChecksumSha256::digest_bytes(b"0123456789");
        let expected_cache = ObjectCachePolicy::PrivateImmutable;
        let expected_expiry = TimestampMillis::new(10_000).expect("timestamp");
        let request = || {
            BeginUploadRequestV1::new(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
            )
            .expect("request")
        };
        let plans = [
            upload_plan(
                key(tenant, video, 2),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                key(TenantId::new(), video, 1),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::DirectSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                ByteSize::new(9).expect("size"),
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                ContentType::parse("video/mp4").expect("content type"),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                ChecksumSha256::digest_bytes(b"different"),
                expected_cache,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                ObjectCachePolicy::PublicImmutable,
                expected_expiry,
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                TimestampMillis::new(9_999).expect("timestamp"),
                request_context.correlation_id(),
            ),
            upload_plan(
                expected_key.clone(),
                UploadMode::BrokeredSinglePut,
                expected_size,
                expected_type.clone(),
                expected_checksum.clone(),
                expected_cache,
                expected_expiry,
                CorrelationId::new(),
            ),
        ];
        let store = DeterministicObjectStore::new(store_capabilities());
        for plan in plans {
            let broker = AdversarialBroker { plan };
            assert_eq!(
                ImmutableStorageService::new(&store, &broker)
                    .begin_upload(request_context, request())
                    .await
                    .expect_err("drifted upload plan"),
                StorageFailure::new(StorageFailureKind::Integrity)
            );
        }
    }
}
