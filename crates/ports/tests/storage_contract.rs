use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, CorrelationId, MediaProfileVersion,
    NormalizedTransformProfile, ObjectRevision, ObjectRole, ScopedObjectKey, StorageFileExtension,
    TenantId, TimestampMillis, TransformProfile, TransformProfileName, VideoId,
    VideoObjectDescriptor,
};
use frame_ports::{
    BeginUploadRequestV1, BrokerUploadId, CompleteUploadRequestV1, CopyObjectRequestV1,
    DeleteObjectDisposition, DeleteObjectRequestV1, DeterministicObjectStore,
    DeterministicUploadBroker, DirectUploadAuthorization, ListObjectsRequestV1, ObjectBodyV1,
    ObjectByteRange, ObjectCachePolicy, ObjectMetadataV1, ObjectRangeBodyV1,
    ObjectStoreCapabilitiesV1, ObjectStoreOperation, ObjectStoreV1, ObjectWriteReceiptV1,
    ProviderEntityTag, ProviderObjectVersion, PutObjectRequestV1, SameOriginUploadPath,
    StorageFailure, StorageFailureKind, StorageListCursor, StorageRequestContext,
    UploadBrokerCapabilitiesV1, UploadBrokerV1, UploadDelivery, UploadMode, UploadPlanV1,
};

fn size(value: u64) -> ByteSize {
    ByteSize::new(value).expect("valid size")
}

fn full_capabilities() -> ObjectStoreCapabilitiesV1 {
    ObjectStoreCapabilitiesV1::full(size(1_000_000), size(1_000_000), 100)
        .expect("valid capabilities")
}

fn broker_capabilities(
    brokered: bool,
    direct: bool,
    multipart: bool,
) -> UploadBrokerCapabilitiesV1 {
    UploadBrokerCapabilitiesV1::new(brokered, direct, multipart, true, size(1_000_000))
        .expect("valid capabilities")
}

fn context(tenant_id: TenantId) -> StorageRequestContext {
    StorageRequestContext::new(tenant_id, CorrelationId::new())
}

fn extension(value: &str) -> StorageFileExtension {
    StorageFileExtension::parse(value).expect("valid extension")
}

fn source_key(tenant_id: TenantId, video_id: VideoId, revision: u64) -> ScopedObjectKey {
    ScopedObjectKey::source(
        tenant_id,
        video_id,
        ObjectRevision::new(revision).expect("revision"),
        VideoObjectDescriptor::Source {
            extension: extension("webm"),
        },
    )
    .expect("source key")
}

fn profile(height: u16) -> TransformProfile {
    TransformProfile::new(
        TransformProfileName::parse("web-preview").expect("profile name"),
        MediaProfileVersion::new(2).expect("profile version"),
        NormalizedTransformProfile::parse(format!("height={height};width=1280"))
            .expect("normalized profile"),
        VideoObjectDescriptor::Preview {
            extension: extension("webm"),
        },
    )
    .expect("transform profile")
}

fn preview_key(tenant_id: TenantId, video_id: VideoId, revision: u64) -> ScopedObjectKey {
    ScopedObjectKey::derivative(
        tenant_id,
        video_id,
        ObjectRevision::new(revision).expect("revision"),
        &profile(720),
    )
    .expect("preview key")
}

fn put_request(key: ScopedObjectKey, bytes: &[u8]) -> PutObjectRequestV1 {
    PutObjectRequestV1::immutable(
        key,
        bytes.to_vec(),
        ContentType::parse("video/webm").expect("content type"),
        ChecksumSha256::digest_bytes(bytes),
        ObjectCachePolicy::PrivateImmutable,
    )
    .expect("put request")
}

#[tokio::test]
async fn full_fake_contract_covers_put_head_get_range_copy_list_and_delete() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let store = DeterministicObjectStore::new(full_capabilities());
    let source = source_key(tenant, video, 1);
    let output = preview_key(tenant, video, 1);
    let bytes = b"0123456789";

    let written = store
        .put(context, put_request(source.clone(), bytes))
        .await
        .expect("put");
    assert_eq!(written.metadata().key(), &source);
    assert_eq!(written.metadata().size(), size(10));
    assert_eq!(written.metadata().content_type().as_str(), "video/webm");
    assert_eq!(
        written.metadata().checksum_sha256(),
        &ChecksumSha256::digest_bytes(bytes)
    );
    assert_eq!(
        written.metadata().correlation_id(),
        context.correlation_id()
    );
    assert!(
        !written
            .metadata()
            .provider_version()
            .expose_for_provider_comparison()
            .is_empty()
    );
    assert!(
        !written
            .metadata()
            .provider_etag()
            .expose_for_provider_comparison()
            .is_empty()
    );
    assert_eq!(
        store.head(context, &source).await.expect("head"),
        written.metadata().clone()
    );
    assert_eq!(
        store.get(context, &source).await.expect("get").bytes(),
        bytes
    );
    let range = store
        .get_range(context, &source, ObjectByteRange::new(2, 7).expect("range"))
        .await
        .expect("range read");
    assert_eq!(range.bytes(), b"23456");
    assert_eq!(range.range(), ObjectByteRange::new(2, 7).expect("range"));

    let wrong_source_version =
        ProviderObjectVersion::parse("fake-v9999999999999999").expect("provider version");
    assert_eq!(
        store
            .copy(
                context,
                CopyObjectRequestV1::immutable(source.clone(), output.clone())
                    .expect("copy")
                    .if_source_version(wrong_source_version),
            )
            .await
            .expect_err("source version is fenced")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    let copied = store
        .copy(
            context,
            CopyObjectRequestV1::immutable(source.clone(), output.clone())
                .expect("copy")
                .if_source_version(written.metadata().provider_version().clone()),
        )
        .await
        .expect("copy");
    assert_eq!(copied.metadata().key(), &output);
    assert_ne!(
        copied.metadata().provider_version(),
        written.metadata().provider_version()
    );

    let first_page = store
        .list(
            context,
            ListObjectsRequestV1::new(tenant, video, None, None, 1).expect("list"),
        )
        .await
        .expect("first page");
    assert_eq!(first_page.items.len(), 1);
    let second_page = store
        .list(
            context,
            ListObjectsRequestV1::new(tenant, video, None, first_page.next_cursor, 1)
                .expect("list"),
        )
        .await
        .expect("second page");
    assert_eq!(second_page.items.len(), 1);
    assert!(second_page.next_cursor.is_none());

    assert_eq!(
        store
            .delete(
                context,
                DeleteObjectRequestV1::if_version(
                    output.clone(),
                    ProviderObjectVersion::parse("wrong-version").expect("version"),
                ),
            )
            .await
            .expect_err("delete version fenced")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    assert_eq!(
        store
            .delete(
                context,
                DeleteObjectRequestV1::if_version(
                    output.clone(),
                    copied.metadata().provider_version().clone(),
                ),
            )
            .await
            .expect("delete"),
        DeleteObjectDisposition::Deleted
    );
    assert_eq!(
        store
            .delete(context, DeleteObjectRequestV1::idempotent(output))
            .await
            .expect("idempotent delete"),
        DeleteObjectDisposition::AlreadyAbsent
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn immutable_create_is_atomic_under_contention() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let request_context = context(tenant);
    let store = Arc::new(DeterministicObjectStore::new(full_capabilities()));
    let key = source_key(tenant, video, 1);
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first = {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let key = key.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store.put(request_context, put_request(key, b"first")).await
        })
    };
    let second = {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let key = key.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store
                .put(request_context, put_request(key, b"second"))
                .await
        })
    };
    barrier.wait().await;
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first task");
    let second = second.expect("second task");
    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(StorageFailure::kind)
            .collect::<Vec<_>>(),
        vec![StorageFailureKind::PreconditionFailed]
    );
    let persisted = store.get(request_context, &key).await.expect("persisted");
    assert!(persisted.bytes() == b"first" || persisted.bytes() == b"second");
}

#[tokio::test]
async fn tenant_boundaries_are_fail_closed_for_every_read_and_mutation() {
    let owner = TenantId::new();
    let attacker = TenantId::new();
    let video = VideoId::new();
    let store = DeterministicObjectStore::new(full_capabilities());
    let key = source_key(owner, video, 1);
    store
        .put(context(owner), put_request(key.clone(), b"private"))
        .await
        .expect("put");
    let attacker_context = context(attacker);

    assert_eq!(
        store
            .head(attacker_context, &key)
            .await
            .expect_err("hidden")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        store
            .get(attacker_context, &key)
            .await
            .expect_err("hidden")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        store
            .get_range(
                attacker_context,
                &key,
                ObjectByteRange::new(0, 1).expect("range"),
            )
            .await
            .expect_err("hidden")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        store
            .delete(
                attacker_context,
                DeleteObjectRequestV1::idempotent(key.clone()),
            )
            .await
            .expect_err("hidden")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        store
            .list(
                attacker_context,
                ListObjectsRequestV1::new(owner, video, None, None, 100).expect("list"),
            )
            .await
            .expect_err("hidden")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert!(CopyObjectRequestV1::immutable(key, preview_key(attacker, video, 1),).is_err());
}

#[tokio::test]
async fn tenant_scope_is_checked_before_capabilities_faults_and_broker_accounting() {
    let owner = TenantId::new();
    let attacker = TenantId::new();
    let video = VideoId::new();
    let key = source_key(owner, video, 1);

    let limited =
        DeterministicObjectStore::new(full_capabilities().without(ObjectStoreOperation::Head));
    assert_eq!(
        limited
            .head(context(attacker), &key)
            .await
            .expect_err("scope must hide capability")
            .kind(),
        StorageFailureKind::NotFound
    );

    let store = DeterministicObjectStore::new(full_capabilities());
    store
        .put(context(owner), put_request(key.clone(), b"private"))
        .await
        .expect("put");
    let injected = StorageFailure::new(StorageFailureKind::Timeout);
    store
        .inject_failure(ObjectStoreOperation::Head, injected.clone())
        .expect("inject");
    assert_eq!(
        store
            .head(context(attacker), &key)
            .await
            .expect_err("scope must not consume fault")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(
        store
            .head(context(owner), &key)
            .await
            .expect_err("owner observes queued fault"),
        injected
    );
    assert_eq!(
        store
            .inject_failure(
                ObjectStoreOperation::ConditionalCreate,
                StorageFailure::new(StorageFailureKind::Timeout),
            )
            .expect_err("capability checks are not injectable")
            .kind(),
        StorageFailureKind::InvalidRequest
    );

    let broker = DeterministicUploadBroker::new(broker_capabilities(true, false, false));
    let request = BeginUploadRequestV1::new(
        key,
        UploadMode::DirectSinglePut,
        size(4),
        ContentType::parse("video/webm").expect("content type"),
        ChecksumSha256::digest_bytes(b"data"),
        ObjectCachePolicy::PrivateImmutable,
        TimestampMillis::new(10_000).expect("timestamp"),
    )
    .expect("request");
    assert_eq!(
        broker
            .begin(context(attacker), request)
            .await
            .expect_err("scope must hide broker capability")
            .kind(),
        StorageFailureKind::NotFound
    );
    assert_eq!(broker.begin_call_count(), 0);
}

#[tokio::test]
async fn version_fences_treat_absence_as_precondition_failure() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let store = DeterministicObjectStore::new(full_capabilities());
    let source = source_key(tenant, video, 1);
    let destination = preview_key(tenant, video, 1);
    let version = ProviderObjectVersion::parse("expected-version").expect("version");
    assert_eq!(
        store
            .copy(
                context(tenant),
                CopyObjectRequestV1::immutable(source.clone(), destination)
                    .expect("copy request")
                    .if_source_version(version.clone()),
            )
            .await
            .expect_err("missing fenced source")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    assert_eq!(
        store
            .delete(
                context(tenant),
                DeleteObjectRequestV1::if_version(source.clone(), version),
            )
            .await
            .expect_err("missing fenced object")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    assert_eq!(
        store
            .delete(context(tenant), DeleteObjectRequestV1::idempotent(source),)
            .await
            .expect("unfenced delete"),
        DeleteObjectDisposition::AlreadyAbsent
    );
}

#[tokio::test]
async fn conditional_copy_and_delete_capabilities_are_independent() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let source = source_key(tenant, video, 1);
    let destination = preview_key(tenant, video, 1);
    let store = DeterministicObjectStore::new(
        full_capabilities().without(ObjectStoreOperation::ConditionalDeleteVersion),
    );
    let written = store
        .put(context(tenant), put_request(source.clone(), b"immutable"))
        .await
        .expect("put");
    store
        .copy(
            context(tenant),
            CopyObjectRequestV1::immutable(source.clone(), destination)
                .expect("copy")
                .if_source_version(written.metadata().provider_version().clone()),
        )
        .await
        .expect("conditional copy remains supported");
    assert_eq!(
        store
            .delete(
                context(tenant),
                DeleteObjectRequestV1::if_version(
                    source.clone(),
                    written.metadata().provider_version().clone(),
                ),
            )
            .await
            .expect_err("conditional delete is independently unsupported")
            .kind(),
        StorageFailureKind::UnsupportedCapability
    );
    assert_eq!(
        store
            .delete(context(tenant), DeleteObjectRequestV1::idempotent(source))
            .await
            .expect("unconditional delete remains supported"),
        DeleteObjectDisposition::Deleted
    );
}

#[tokio::test]
async fn integrity_failure_is_terminal_and_leaves_no_partial_object() {
    let tenant = TenantId::new();
    let store = DeterministicObjectStore::new(full_capabilities());
    let request = PutObjectRequestV1::immutable(
        source_key(tenant, VideoId::new(), 1),
        b"actual".to_vec(),
        ContentType::parse("video/webm").expect("content type"),
        ChecksumSha256::digest_bytes(b"different"),
        ObjectCachePolicy::PrivateImmutable,
    )
    .expect("request");
    let error = store
        .put(context(tenant), request)
        .await
        .expect_err("integrity mismatch");
    assert_eq!(error.kind(), StorageFailureKind::Integrity);
    assert!(!error.retryable());
    assert_eq!(store.object_count().expect("count"), 0);
}

#[tokio::test]
async fn capabilities_and_scripted_failure_taxonomy_are_exact_and_safe() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let key = source_key(tenant, video, 1);
    let limited = full_capabilities().without(ObjectStoreOperation::Range);
    let store = DeterministicObjectStore::new(limited);
    assert_eq!(
        store
            .get_range(
                context(tenant),
                &key,
                ObjectByteRange::new(0, 1).expect("range"),
            )
            .await
            .expect_err("unsupported")
            .kind(),
        StorageFailureKind::UnsupportedCapability
    );

    let store = DeterministicObjectStore::new(full_capabilities());
    store
        .put(context(tenant), put_request(key.clone(), b"content"))
        .await
        .expect("put");
    let throttled =
        StorageFailure::throttled(frame_domain::DurationMillis::new(250).expect("duration"));
    store
        .inject_failure(ObjectStoreOperation::Head, throttled.clone())
        .expect("inject");
    let observed = store.head(context(tenant), &key).await.expect_err("fault");
    assert_eq!(observed, throttled);
    assert!(observed.retryable());
    assert_eq!(observed.retry_after().expect("retry after").get(), 250);
    assert!(!format!("{observed:?}").contains("content"));
}

#[tokio::test]
async fn broker_plan_completion_replay_conflict_and_abort_are_deterministic() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = context(tenant);
    let key = source_key(tenant, video, 1);
    let bytes = b"brokered";
    let broker = DeterministicUploadBroker::new(broker_capabilities(true, false, false));
    let plan = broker
        .begin(
            context,
            BeginUploadRequestV1::new(
                key.clone(),
                UploadMode::BrokeredSinglePut,
                size(u64::try_from(bytes.len()).expect("length")),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(bytes),
                ObjectCachePolicy::PrivateImmutable,
                TimestampMillis::new(10_000).expect("timestamp"),
            )
            .expect("begin request"),
        )
        .await
        .expect("begin");
    assert!(matches!(plan.delivery(), UploadDelivery::Brokered { .. }));
    assert_eq!(plan.key(), &key);

    let store = DeterministicObjectStore::new(full_capabilities());
    let receipt = store
        .put(context, put_request(key, bytes))
        .await
        .expect("put");
    let completion = || CompleteUploadRequestV1::new(plan.id().clone(), receipt.clone());
    assert_eq!(
        broker
            .complete(context, completion())
            .await
            .expect("complete"),
        receipt
    );
    assert_eq!(
        broker
            .complete(context, completion())
            .await
            .expect("replay"),
        receipt
    );
    let other_receipt = store
        .put(
            context,
            put_request(source_key(tenant, video, 2), b"different"),
        )
        .await
        .expect("other put");
    assert_eq!(
        broker
            .complete(
                context,
                CompleteUploadRequestV1::new(plan.id().clone(), other_receipt),
            )
            .await
            .expect_err("changed replay")
            .kind(),
        StorageFailureKind::PreconditionFailed
    );
    broker.abort(context, plan.id()).await.expect("abort");
    assert_eq!(
        broker
            .complete(context, completion())
            .await
            .expect("completed upload survives abort"),
        receipt
    );
    broker
        .abort(context, plan.id())
        .await
        .expect("idempotent abort");
}

#[tokio::test]
async fn broker_abort_hides_tenant_existence_and_removes_only_pending_owner_uploads() {
    let owner = TenantId::new();
    let attacker = TenantId::new();
    let video = VideoId::new();
    let owner_context = context(owner);
    let key = source_key(owner, video, 1);
    let bytes = b"data";
    let broker = DeterministicUploadBroker::new(broker_capabilities(true, false, false));
    let plan = broker
        .begin(
            owner_context,
            BeginUploadRequestV1::new(
                key.clone(),
                UploadMode::BrokeredSinglePut,
                size(4),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(bytes),
                ObjectCachePolicy::PrivateImmutable,
                TimestampMillis::new(10_000).expect("timestamp"),
            )
            .expect("request"),
        )
        .await
        .expect("plan");
    let unknown = BrokerUploadId::parse("unknown-upload-1").expect("id");
    broker
        .abort(context(attacker), &unknown)
        .await
        .expect("unknown abort");
    broker
        .abort(context(attacker), plan.id())
        .await
        .expect("cross-tenant abort is indistinguishable");

    let store = DeterministicObjectStore::new(full_capabilities());
    let receipt = store
        .put(owner_context, put_request(key, bytes))
        .await
        .expect("put");
    broker
        .complete(
            owner_context,
            CompleteUploadRequestV1::new(plan.id().clone(), receipt),
        )
        .await
        .expect("attacker did not remove upload");

    let pending = broker
        .begin(
            context(owner),
            BeginUploadRequestV1::new(
                source_key(owner, video, 2),
                UploadMode::BrokeredSinglePut,
                size(4),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(bytes),
                ObjectCachePolicy::PrivateImmutable,
                TimestampMillis::new(10_000).expect("timestamp"),
            )
            .expect("request"),
        )
        .await
        .expect("plan");
    broker
        .abort(context(owner), pending.id())
        .await
        .expect("owner abort");
    assert_eq!(
        broker
            .complete(
                context(owner),
                CompleteUploadRequestV1::new(
                    pending.id().clone(),
                    ObjectWriteReceiptV1::new(
                        ObjectMetadataV1::new(
                            source_key(owner, video, 2),
                            size(4),
                            ContentType::parse("video/webm").expect("content type"),
                            ChecksumSha256::digest_bytes(bytes),
                            ProviderObjectVersion::parse("version-2").expect("version"),
                            ProviderEntityTag::parse("etag-2").expect("etag"),
                            ObjectCachePolicy::PrivateImmutable,
                            TimestampMillis::new(1).expect("timestamp"),
                            context(owner).correlation_id(),
                        )
                        .expect("metadata"),
                    ),
                ),
            )
            .await
            .expect_err("aborted pending upload")
            .kind(),
        StorageFailureKind::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broker_complete_and_abort_are_linearizable_under_contention() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let request_context = context(tenant);
    let key = source_key(tenant, video, 1);
    let bytes = b"data";
    let broker = Arc::new(DeterministicUploadBroker::new(broker_capabilities(
        true, false, false,
    )));
    let plan = broker
        .begin(
            request_context,
            BeginUploadRequestV1::new(
                key.clone(),
                UploadMode::BrokeredSinglePut,
                size(4),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(bytes),
                ObjectCachePolicy::PrivateImmutable,
                TimestampMillis::new(10_000).expect("timestamp"),
            )
            .expect("request"),
        )
        .await
        .expect("plan");
    let store = DeterministicObjectStore::new(full_capabilities());
    let receipt = store
        .put(request_context, put_request(key, bytes))
        .await
        .expect("receipt");
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let completion = {
        let broker = Arc::clone(&broker);
        let barrier = Arc::clone(&barrier);
        let id = plan.id().clone();
        let receipt = receipt.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            broker
                .complete(request_context, CompleteUploadRequestV1::new(id, receipt))
                .await
        })
    };
    let abort = {
        let broker = Arc::clone(&broker);
        let barrier = Arc::clone(&barrier);
        let id = plan.id().clone();
        tokio::spawn(async move {
            barrier.wait().await;
            broker.abort(request_context, &id).await
        })
    };
    barrier.wait().await;
    let (completion, abort) = tokio::join!(completion, abort);
    let completion = completion.expect("completion task");
    abort.expect("abort task").expect("abort result");
    match completion {
        Ok(completed) => {
            assert_eq!(completed, receipt);
            assert_eq!(
                broker
                    .complete(
                        request_context,
                        CompleteUploadRequestV1::new(plan.id().clone(), receipt.clone()),
                    )
                    .await
                    .expect("completed state survives abort"),
                receipt
            );
        }
        Err(error) => assert_eq!(error.kind(), StorageFailureKind::NotFound),
    }
}

#[tokio::test]
async fn direct_authorization_is_validated_and_never_formatted() {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), "Bearer super-secret".to_owned());
    let authorization = DirectUploadAuthorization::new(
        "https://upload.invalid/object?signature=super-secret",
        headers,
    )
    .expect("authorization");
    let formatted = format!("{authorization:?}");
    assert!(!formatted.contains("super-secret"));
    assert!(!formatted.contains("upload.invalid"));
    assert!(DirectUploadAuthorization::new("http://insecure.invalid", BTreeMap::new()).is_err());
    let credential_url = ["https://user", "test-only-password@upload.invalid/object"].join(":");
    assert!(DirectUploadAuthorization::new(credential_url, BTreeMap::new()).is_err());
    for malformed in [
        "https://:443/object",
        "https://bad..host/object",
        "https://upload.invalid:0/object",
        "https://uplöad.invalid/object",
    ] {
        assert!(
            DirectUploadAuthorization::new(malformed, BTreeMap::new()).is_err(),
            "{malformed}"
        );
    }
    let mut injected = BTreeMap::new();
    injected.insert("x-test".to_owned(), "value\r\nsecond: injected".to_owned());
    assert!(DirectUploadAuthorization::new("https://upload.invalid/object", injected).is_err());
    for value in ["value\tcontinued", "value\0hidden", "value\u{7f}"] {
        let mut headers = BTreeMap::new();
        headers.insert("x-test".to_owned(), value.to_owned());
        assert!(
            DirectUploadAuthorization::new("https://upload.invalid/object", headers).is_err(),
            "control characters must be rejected"
        );
    }
    let mut duplicates = BTreeMap::new();
    duplicates.insert("X-Frame-Upload".to_owned(), "first".to_owned());
    duplicates.insert("x-frame-upload".to_owned(), "second".to_owned());
    assert!(DirectUploadAuthorization::new("https://upload.invalid/object", duplicates).is_err());

    let tenant = TenantId::new();
    let broker = DeterministicUploadBroker::new(broker_capabilities(false, true, false));
    let plan = broker
        .begin(
            context(tenant),
            BeginUploadRequestV1::new(
                source_key(tenant, VideoId::new(), 1),
                UploadMode::DirectSinglePut,
                size(4),
                ContentType::parse("video/webm").expect("content type"),
                ChecksumSha256::digest_bytes(b"data"),
                ObjectCachePolicy::PrivateImmutable,
                TimestampMillis::new(10_000).expect("timestamp"),
            )
            .expect("request"),
        )
        .await
        .expect("plan");
    let formatted = format!("{plan:?}");
    assert!(!formatted.contains("upload.invalid"));
    assert!(!formatted.contains("x-frame-upload"));
    let redacted_path = SameOriginUploadPath::parse("/api/storage/sensitive-path").expect("path");
    assert!(!format!("{redacted_path:?}").contains("sensitive-path"));
    assert!(SameOriginUploadPath::parse("/api//storage").is_err());
    assert!(SameOriginUploadPath::parse("//provider.example/upload").is_err());
    assert!(SameOriginUploadPath::parse("/api/../secret").is_err());
}

#[tokio::test]
async fn object_debug_surfaces_redact_raw_payload_bytes() {
    let tenant = TenantId::new();
    let key = source_key(tenant, VideoId::new(), 1);
    let bytes = b"raw-object-secret";
    let request = put_request(key.clone(), bytes);
    assert!(!format!("{request:?}").contains("raw-object-secret"));
    let store = DeterministicObjectStore::new(full_capabilities());
    store.put(context(tenant), request).await.expect("put");
    let body = store.get(context(tenant), &key).await.expect("get");
    assert!(!format!("{body:?}").contains("raw-object-secret"));
    let range = store
        .get_range(
            context(tenant),
            &key,
            ObjectByteRange::new(0, 10).expect("range"),
        )
        .await
        .expect("range");
    assert!(!format!("{range:?}").contains("raw-object"));
}

#[test]
fn generated_key_space_does_not_collide_across_scope_revision_or_profile() {
    let tenants = [TenantId::new(), TenantId::new()];
    let videos = [VideoId::new(), VideoId::new()];
    let mut keys = BTreeSet::new();
    for tenant in tenants {
        for video in videos {
            for revision in 1..=16 {
                let source = source_key(tenant, video, revision);
                assert!(keys.insert(source.as_str().to_owned()));
                for height in [720, 1080] {
                    let derivative = ScopedObjectKey::derivative(
                        tenant,
                        video,
                        ObjectRevision::new(revision).expect("revision"),
                        &profile(height),
                    )
                    .expect("derivative");
                    assert!(keys.insert(derivative.as_str().to_owned()));
                }
            }
        }
    }
    assert_eq!(keys.len(), 2 * 2 * 16 * 3);
}

#[test]
fn broker_identifiers_and_failure_messages_do_not_accept_or_expose_secrets() {
    assert!(BrokerUploadId::parse("../../secret").is_err());
    for kind in [
        StorageFailureKind::NotFound,
        StorageFailureKind::PreconditionFailed,
        StorageFailureKind::Throttled,
        StorageFailureKind::Unauthorized,
        StorageFailureKind::QuotaExceeded,
        StorageFailureKind::Timeout,
        StorageFailureKind::Integrity,
        StorageFailureKind::Unavailable,
        StorageFailureKind::UnsupportedCapability,
        StorageFailureKind::InvalidRequest,
    ] {
        let failure = StorageFailure::new(kind);
        assert!(!failure.safe_message().contains("provider"));
        assert_eq!(
            failure.retryable(),
            matches!(
                kind,
                StorageFailureKind::Throttled
                    | StorageFailureKind::Timeout
                    | StorageFailureKind::Unavailable
            )
        );
    }
}

#[test]
fn external_adapters_can_construct_validated_result_contracts() {
    let tenant = TenantId::new();
    let key = source_key(tenant, VideoId::new(), 1);
    let bytes = b"data".to_vec();
    let correlation_id = CorrelationId::new();
    let metadata = ObjectMetadataV1::new(
        key.clone(),
        size(4),
        ContentType::parse("video/webm").expect("content type"),
        ChecksumSha256::digest_bytes(&bytes),
        ProviderObjectVersion::parse("external-version-1").expect("version"),
        ProviderEntityTag::parse("external-etag-1").expect("etag"),
        ObjectCachePolicy::PrivateImmutable,
        TimestampMillis::new(1).expect("timestamp"),
        correlation_id,
    )
    .expect("metadata");
    let receipt = ObjectWriteReceiptV1::new(metadata.clone());
    assert_eq!(receipt.metadata(), &metadata);
    assert_eq!(
        ObjectBodyV1::new(metadata.clone(), bytes.clone())
            .expect("body")
            .bytes(),
        bytes
    );
    assert_eq!(
        ObjectBodyV1::new(metadata.clone(), b"evil".to_vec())
            .expect_err("integrity")
            .kind(),
        StorageFailureKind::Integrity
    );
    let range = ObjectRangeBodyV1::new(
        metadata,
        b"at".to_vec(),
        ObjectByteRange::new(1, 3).expect("range"),
    )
    .expect("range body");
    assert_eq!(range.bytes(), b"at");
    let delivery = UploadDelivery::Brokered {
        path: SameOriginUploadPath::parse("/api/storage/uploads/external").expect("path"),
    };
    let plan = UploadPlanV1::new(
        BrokerUploadId::parse("external-upload-1").expect("id"),
        key,
        UploadMode::BrokeredSinglePut,
        delivery,
        size(4),
        ContentType::parse("video/webm").expect("content type"),
        ChecksumSha256::digest_bytes(&bytes),
        ObjectCachePolicy::PrivateImmutable,
        TimestampMillis::new(10).expect("timestamp"),
        correlation_id,
    )
    .expect("plan");
    assert_eq!(plan.expected_size(), size(4));
    assert!(ProviderObjectVersion::parse("s3-style+/version==").is_ok());
}

#[test]
fn external_adapters_can_read_every_private_request_field() {
    let tenant = TenantId::new();
    let video = VideoId::new();
    let source = source_key(tenant, video, 1);
    let destination = preview_key(tenant, video, 1);
    let expected_version = ProviderObjectVersion::parse("external-version").expect("version");
    let copy = CopyObjectRequestV1::immutable(source.clone(), destination.clone())
        .expect("copy")
        .if_source_version(expected_version.clone());
    assert_eq!(copy.source(), &source);
    assert_eq!(copy.destination(), &destination);
    assert_eq!(copy.expected_source_version(), Some(&expected_version));

    let delete = DeleteObjectRequestV1::if_version(source.clone(), expected_version.clone());
    assert_eq!(delete.key(), &source);
    assert_eq!(delete.expected_version(), Some(&expected_version));

    let cursor = StorageListCursor::parse(source.as_str()).expect("cursor");
    let list = ListObjectsRequestV1::new(
        tenant,
        video,
        Some(ObjectRole::Source),
        Some(cursor.clone()),
        17,
    )
    .expect("list");
    assert_eq!(list.tenant_id(), tenant);
    assert_eq!(list.video_id(), video);
    assert_eq!(list.role(), Some(ObjectRole::Source));
    assert_eq!(list.cursor(), Some(&cursor));
    assert_eq!(list.limit(), 17);

    let checksum = ChecksumSha256::digest_bytes(b"data");
    let expiry = TimestampMillis::new(10_000).expect("timestamp");
    let begin = BeginUploadRequestV1::new(
        source.clone(),
        UploadMode::BrokeredSinglePut,
        size(4),
        ContentType::parse("video/webm").expect("content type"),
        checksum.clone(),
        ObjectCachePolicy::PrivateImmutable,
        expiry,
    )
    .expect("begin");
    assert_eq!(begin.key(), &source);
    assert_eq!(begin.mode(), UploadMode::BrokeredSinglePut);
    assert_eq!(begin.expected_size(), size(4));
    assert_eq!(begin.content_type().as_str(), "video/webm");
    assert_eq!(begin.checksum_sha256(), &checksum);
    assert_eq!(begin.cache_policy(), ObjectCachePolicy::PrivateImmutable);
    assert_eq!(begin.expires_at(), expiry);

    let correlation = CorrelationId::new();
    let receipt = ObjectWriteReceiptV1::new(
        ObjectMetadataV1::new(
            source,
            size(4),
            ContentType::parse("video/webm").expect("content type"),
            checksum,
            expected_version,
            ProviderEntityTag::parse("external-etag").expect("etag"),
            ObjectCachePolicy::PrivateImmutable,
            TimestampMillis::new(1).expect("timestamp"),
            correlation,
        )
        .expect("metadata"),
    );
    let id = BrokerUploadId::parse("external-upload-accessor").expect("id");
    let complete = CompleteUploadRequestV1::new(id.clone(), receipt.clone());
    assert_eq!(complete.id(), &id);
    assert_eq!(complete.receipt(), &receipt);
}
