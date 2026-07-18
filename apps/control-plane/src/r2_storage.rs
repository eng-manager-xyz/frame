//! Production Cloudflare R2 Worker-binding adapter for the v1 object-store port.
//!
//! Every operation executes against `worker::Bucket`; immutable conditions,
//! tenant scope, checksums, metadata, and provider cursors stay typed.

use std::collections::HashMap;

use async_trait::async_trait;
use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, CorrelationId, ObjectRevision, ScopedObjectKey,
    StorageFileExtension, TenantId, TimestampMillis, VideoId, VideoObjectDescriptor,
};
use frame_ports::{
    CopyObjectRequestV1, DeleteObjectDisposition, DeleteObjectRequestV1, ListObjectsPageV1,
    ListObjectsRequestV1, ObjectBodyV1, ObjectByteRange, ObjectCachePolicy, ObjectMetadataV1,
    ObjectRangeBodyV1, ObjectStoreCapabilitiesV1, ObjectStoreOperation, ObjectStoreV1,
    ObjectWriteReceiptV1, ProviderEntityTag, ProviderObjectVersion, PutObjectRequestV1,
    StorageFailure, StorageFailureKind, StorageListCursor, StorageRequestContext,
};
use worker::{
    Bucket, Conditional, Env, HttpMetadata, Include, Method, Range, Request, Response,
    send::IntoSendFuture,
};

const MAX_OBJECT_BYTES: u64 = 32 * 1_024 * 1_024;
const MAX_RANGE_BYTES: u64 = 16 * 1_024 * 1_024;
const META_CORRELATION: &str = "frame-correlation-id";
const META_CACHE_POLICY: &str = "frame-cache-policy";

/// Runs the complete adapter contract against Wrangler's local R2 binding.
/// The route exposing this function is loopback-only and absent in production.
pub async fn local_conformance_response(request: Request, env: &Env) -> worker::Result<Response> {
    if request.method() != Method::Post {
        return Response::error("method not allowed", 405);
    }
    let bucket = env.bucket("RECORDINGS")?;
    let database = env.d1("DB")?;
    let adapter = R2ObjectStoreV1::new(&bucket)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    run_local_contract(&adapter)
        .await
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    crate::r2_multipart::run_local_contract(&bucket, &database)
        .await
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    Response::from_json(&serde_json::json!({
        "schema_version": 1,
        "adapter": "cloudflare_r2_worker_binding_v1",
        "operations": ["put", "head", "get", "range", "copy", "delete", "list"],
        "multipart_operations": [
            "create", "lookup", "list_parts", "put_part", "complete", "abort",
            "stale_cleanup", "head", "range"
        ],
        "multipart_conditions": [
            "durable_pre_provider_create_claim", "provider_handle_reconciliation",
            "leased_part_write_claim", "strict_completion_linearization",
            "completion_abort_exclusion", "expired_open_completion_rejected"
        ],
        "conditions": [
            "immutable_create", "exact_replay", "conditional_source_version",
            "conditional_delete_unsupported", "cross_tenant_not_found"
        ],
        "status": "passed"
    }))
}

async fn run_local_contract(adapter: &R2ObjectStoreV1<'_>) -> Result<(), StorageFailure> {
    if !adapter
        .capabilities()
        .supports(ObjectStoreOperation::ConditionalSourceVersion)
        || adapter
            .capabilities()
            .supports(ObjectStoreOperation::ConditionalDeleteVersion)
    {
        return Err(integrity());
    }
    let tenant = TenantId::new();
    let video = VideoId::new();
    let context = StorageRequestContext::new(tenant, CorrelationId::new());
    let key = ScopedObjectKey::source(
        tenant,
        video,
        ObjectRevision::new(1).map_err(|_| invalid())?,
        VideoObjectDescriptor::Source {
            extension: StorageFileExtension::parse("bin").map_err(|_| invalid())?,
        },
    )
    .map_err(|_| invalid())?;
    let copy_key = ScopedObjectKey::source(
        tenant,
        video,
        ObjectRevision::new(2).map_err(|_| invalid())?,
        VideoObjectDescriptor::Source {
            extension: StorageFileExtension::parse("bin").map_err(|_| invalid())?,
        },
    )
    .map_err(|_| invalid())?;
    let bytes = b"frame-r2-binding-contract-v1".to_vec();
    let checksum = ChecksumSha256::digest_bytes(&bytes);
    let content_type = ContentType::parse("application/octet-stream").map_err(|_| invalid())?;
    let put = PutObjectRequestV1::immutable(
        key.clone(),
        bytes.clone(),
        content_type.clone(),
        checksum.clone(),
        ObjectCachePolicy::NoStore,
    )?;
    let first = adapter.put(context, put.clone()).await?;
    let replay = adapter.put(context, put.clone()).await?;
    let head = adapter.head(context, &key).await?;
    if first.metadata() != replay.metadata()
        || head.key() != &key
        || head.size().get() != u64::try_from(bytes.len()).map_err(|_| invalid())?
        || head.content_type() != &content_type
        || head.checksum_sha256() != &checksum
        || head.cache_policy() != ObjectCachePolicy::NoStore
        || head.correlation_id() != context.correlation_id()
        || head
            .provider_version()
            .expose_for_provider_comparison()
            .is_empty()
        || head
            .provider_etag()
            .expose_for_provider_comparison()
            .is_empty()
        || adapter.get(context, &key).await?.bytes() != bytes
        || adapter
            .get_range(context, &key, ObjectByteRange::new(6, 16)?)
            .await?
            .bytes()
            != &bytes[6..16]
    {
        return Err(integrity());
    }
    let conflicting = PutObjectRequestV1::immutable(
        key.clone(),
        b"different".to_vec(),
        content_type.clone(),
        ChecksumSha256::digest_bytes(b"different"),
        ObjectCachePolicy::NoStore,
    )?;
    if !matches!(
        adapter.put(context, conflicting).await,
        Err(error) if error.kind() == StorageFailureKind::PreconditionFailed
    ) {
        return Err(integrity());
    }
    if !matches!(
        adapter
            .copy(
                context,
                CopyObjectRequestV1::immutable(key.clone(), copy_key.clone())?.if_source_version(
                    ProviderObjectVersion::parse("definitely-wrong")?,
                ),
            )
            .await,
        Err(error) if error.kind() == StorageFailureKind::PreconditionFailed
    ) {
        return Err(integrity());
    }
    let copied = adapter
        .copy(
            context,
            CopyObjectRequestV1::immutable(key.clone(), copy_key.clone())?
                .if_source_version(first.metadata().provider_version().clone()),
        )
        .await?;
    if copied.metadata().key() != &copy_key
        || copied.metadata().checksum_sha256() != &checksum
        || copied.metadata().content_type() != &content_type
        || copied.metadata().cache_policy() != ObjectCachePolicy::NoStore
        || copied.metadata().correlation_id() != context.correlation_id()
    {
        return Err(integrity());
    }
    let page_one = adapter
        .list(
            context,
            ListObjectsRequestV1::new(tenant, video, None, None, 1)?,
        )
        .await?;
    let page_two = adapter
        .list(
            context,
            ListObjectsRequestV1::new(tenant, video, None, page_one.next_cursor.clone(), 1)?,
        )
        .await?;
    if page_one.items.len() != 1
        || page_one.next_cursor.is_none()
        || page_two.items.len() != 1
        || page_two.next_cursor.is_some()
        || !page_one
            .items
            .iter()
            .chain(&page_two.items)
            .any(|metadata| metadata.key() == &key)
        || !page_one
            .items
            .iter()
            .chain(&page_two.items)
            .any(|metadata| metadata.key() == &copy_key)
    {
        return Err(integrity());
    }
    if !matches!(
        adapter
            .delete(
                context,
                DeleteObjectRequestV1::if_version(
                    copy_key.clone(),
                    ProviderObjectVersion::parse("definitely-wrong")?,
                ),
            )
            .await,
        Err(error) if error.kind() == StorageFailureKind::UnsupportedCapability
    ) {
        return Err(integrity());
    }

    let wrong_context = StorageRequestContext::new(TenantId::new(), CorrelationId::new());
    require_failure_kind(
        adapter.put(wrong_context, put).await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter.head(wrong_context, &key).await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter.get(wrong_context, &key).await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter
            .get_range(wrong_context, &key, ObjectByteRange::new(0, 1)?)
            .await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter
            .copy(
                wrong_context,
                CopyObjectRequestV1::immutable(key.clone(), copy_key.clone())?,
            )
            .await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter
            .delete(
                wrong_context,
                DeleteObjectRequestV1::idempotent(copy_key.clone()),
            )
            .await,
        StorageFailureKind::NotFound,
    )?;
    require_failure_kind(
        adapter
            .list(
                wrong_context,
                ListObjectsRequestV1::new(tenant, video, None, None, 100)?,
            )
            .await,
        StorageFailureKind::NotFound,
    )?;
    if adapter.head(context, &key).await?.provider_version() != first.metadata().provider_version()
        || adapter.head(context, &copy_key).await?.provider_version()
            != copied.metadata().provider_version()
    {
        return Err(integrity());
    }
    if adapter
        .delete(context, DeleteObjectRequestV1::idempotent(copy_key.clone()))
        .await?
        != DeleteObjectDisposition::Deleted
        || adapter
            .delete(context, DeleteObjectRequestV1::idempotent(copy_key))
            .await?
            != DeleteObjectDisposition::AlreadyAbsent
    {
        return Err(integrity());
    }
    adapter
        .delete(context, DeleteObjectRequestV1::idempotent(key))
        .await?;
    Ok(())
}

#[derive(Debug)]
pub struct R2ObjectStoreV1<'a> {
    bucket: &'a Bucket,
    capabilities: ObjectStoreCapabilitiesV1,
}

impl<'a> R2ObjectStoreV1<'a> {
    pub fn new(bucket: &'a Bucket) -> Result<Self, StorageFailure> {
        Ok(Self {
            bucket,
            capabilities: ObjectStoreCapabilitiesV1::full(
                ByteSize::new(MAX_OBJECT_BYTES).map_err(|_| invalid())?,
                ByteSize::new(MAX_RANGE_BYTES).map_err(|_| invalid())?,
                100,
            )?
            .without(ObjectStoreOperation::ConditionalDeleteVersion),
        })
    }

    fn authorize(
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<(), StorageFailure> {
        (context.tenant_id() == key.tenant_id())
            .then_some(())
            .ok_or_else(not_found)
    }

    fn metadata(
        object: &worker::Object,
        key: &ScopedObjectKey,
    ) -> Result<ObjectMetadataV1, StorageFailure> {
        if object.key() != key.as_str() || object.size() == 0 {
            return Err(integrity());
        }
        let http = object.http_metadata();
        if http.content_encoding.is_some() {
            return Err(integrity());
        }
        let content_type = http
            .content_type
            .as_deref()
            .and_then(|value| ContentType::parse(value).ok())
            .ok_or_else(integrity)?;
        let checksum = object
            .checksum()
            .sha256
            .as_deref()
            .map(hex)
            .and_then(|value| ChecksumSha256::parse(value).ok())
            .ok_or_else(integrity)?;
        let custom = object.custom_metadata().map_err(map_worker_error)?;
        let correlation = custom
            .get(META_CORRELATION)
            .and_then(|value| CorrelationId::parse(value).ok())
            .ok_or_else(integrity)?;
        let cache_policy = custom
            .get(META_CACHE_POLICY)
            .and_then(|value| parse_cache_policy(value))
            .ok_or_else(integrity)?;
        ObjectMetadataV1::new(
            key.clone(),
            ByteSize::new(object.size()).map_err(|_| integrity())?,
            content_type,
            checksum,
            ProviderObjectVersion::parse(object.version()).map_err(|_| integrity())?,
            ProviderEntityTag::parse(object.etag()).map_err(|_| integrity())?,
            cache_policy,
            TimestampMillis::new(
                i64::try_from(object.uploaded().as_millis()).map_err(|_| integrity())?,
            )
            .map_err(|_| integrity())?,
            correlation,
        )
    }

    async fn exact_existing(
        &self,
        context: StorageRequestContext,
        request: &PutObjectRequestV1,
    ) -> Result<Option<ObjectMetadataV1>, StorageFailure> {
        let Some(object) = self
            .bucket
            .head(request.key().as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?
        else {
            return Ok(None);
        };
        let metadata = Self::metadata(&object, request.key())?;
        let exact = metadata.size().get()
            == u64::try_from(request.bytes().len()).map_err(|_| invalid())?
            && metadata.content_type() == request.content_type()
            && metadata.checksum_sha256() == request.checksum_sha256()
            && metadata.cache_policy() == request.cache_policy()
            && metadata.correlation_id() == context.correlation_id();
        if exact {
            Ok(Some(metadata))
        } else {
            Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
        }
    }
}

#[async_trait]
impl ObjectStoreV1 for R2ObjectStoreV1<'_> {
    fn capabilities(&self) -> ObjectStoreCapabilitiesV1 {
        self.capabilities
    }

    async fn put(
        &self,
        context: StorageRequestContext,
        request: PutObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        let size = u64::try_from(request.bytes().len()).map_err(|_| invalid())?;
        if size == 0
            || size > self.capabilities.max_object_size().get()
            || &ChecksumSha256::digest_bytes(request.bytes()) != request.checksum_sha256()
        {
            return Err(invalid());
        }
        if let Some(metadata) = self.exact_existing(context, &request).await? {
            return Ok(ObjectWriteReceiptV1::new(metadata));
        }
        let http = HttpMetadata {
            content_type: Some(request.content_type().as_str().to_owned()),
            content_disposition: Some("attachment".into()),
            cache_control: Some(cache_control(request.cache_policy()).into()),
            ..HttpMetadata::default()
        };
        let custom = HashMap::from([
            (
                META_CORRELATION.into(),
                context.correlation_id().to_string(),
            ),
            (
                META_CACHE_POLICY.into(),
                cache_policy_name(request.cache_policy()).into(),
            ),
        ]);
        let checksum = decode_hex_32(request.checksum_sha256().as_str()).ok_or_else(invalid)?;
        let applied = self
            .bucket
            .put(request.key().as_str(), request.bytes().to_vec())
            .http_metadata(http)
            .custom_metadata(custom)
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let metadata = match applied {
            Some(object) => Self::metadata(&object, request.key())?,
            None => self
                .exact_existing(context, &request)
                .await?
                .ok_or_else(|| StorageFailure::new(StorageFailureKind::PreconditionFailed))?,
        };
        if metadata.size().get() != size
            || metadata.content_type() != request.content_type()
            || metadata.checksum_sha256() != request.checksum_sha256()
        {
            return Err(integrity());
        }
        Ok(ObjectWriteReceiptV1::new(metadata))
    }

    async fn head(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectMetadataV1, StorageFailure> {
        Self::authorize(context, key)?;
        let object = self
            .bucket
            .head(key.as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(not_found)?;
        Self::metadata(&object, key)
    }

    async fn get(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectBodyV1, StorageFailure> {
        Self::authorize(context, key)?;
        let object = self
            .bucket
            .get(key.as_str())
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(not_found)?;
        let metadata = Self::metadata(&object, key)?;
        let bytes = object
            .body()
            .ok_or_else(integrity)?
            .bytes()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        ObjectBodyV1::new(metadata, bytes)
    }

    async fn get_range(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
        range: ObjectByteRange,
    ) -> Result<ObjectRangeBodyV1, StorageFailure> {
        Self::authorize(context, key)?;
        if range.length() > self.capabilities.max_range_size().get() {
            return Err(invalid());
        }
        let object = self
            .bucket
            .get(key.as_str())
            .range(Range::OffsetWithLength {
                offset: range.start(),
                length: range.length(),
            })
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?
            .ok_or_else(not_found)?;
        let metadata = Self::metadata(&object, key)?;
        let bytes = object
            .body()
            .ok_or_else(integrity)?
            .bytes()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        ObjectRangeBodyV1::new(metadata, bytes, range)
    }

    async fn copy(
        &self,
        context: StorageRequestContext,
        request: CopyObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        Self::authorize(context, request.source())?;
        Self::authorize(context, request.destination())?;
        let source = self.get(context, request.source()).await?;
        if request
            .expected_source_version()
            .is_some_and(|expected| expected != source.metadata().provider_version())
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        self.put(
            context,
            PutObjectRequestV1::immutable(
                request.destination().clone(),
                source.bytes().to_vec(),
                source.metadata().content_type().clone(),
                source.metadata().checksum_sha256().clone(),
                source.metadata().cache_policy(),
            )?,
        )
        .await
    }

    async fn delete(
        &self,
        context: StorageRequestContext,
        request: DeleteObjectRequestV1,
    ) -> Result<DeleteObjectDisposition, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.expected_version().is_some() {
            // worker-rs exposes no conditional delete on an R2 Bucket. A
            // HEAD followed by unconditional delete would permit a
            // delete/recreate race, so this adapter fails before provider I/O.
            return Err(StorageFailure::unsupported());
        }
        let Some(existing) = self
            .bucket
            .head(request.key().as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?
        else {
            return Ok(DeleteObjectDisposition::AlreadyAbsent);
        };
        Self::metadata(&existing, request.key())?;
        self.bucket
            .delete(request.key().as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?;
        if self
            .bucket
            .head(request.key().as_str())
            .into_send()
            .await
            .map_err(map_worker_error)?
            .is_some()
        {
            return Err(StorageFailure::new(StorageFailureKind::Unavailable));
        }
        Ok(DeleteObjectDisposition::Deleted)
    }

    async fn list(
        &self,
        context: StorageRequestContext,
        request: ListObjectsRequestV1,
    ) -> Result<ListObjectsPageV1, StorageFailure> {
        if context.tenant_id() != request.tenant_id() {
            return Err(not_found());
        }
        let mut prefix = format!(
            "tenants/{}/videos/{}/v{}/",
            request.tenant_id(),
            request.video_id(),
            frame_domain::STORAGE_KEY_SCHEMA_VERSION
        );
        if let Some(role) = request.role() {
            prefix.push_str(role.path_segment());
            prefix.push('/');
        }
        let mut builder = self
            .bucket
            .list()
            .limit(u32::from(request.limit()))
            .prefix(prefix)
            .include(vec![Include::HttpMetadata, Include::CustomMetadata]);
        if let Some(cursor) = request.cursor() {
            builder =
                builder.cursor(decode_cursor(cursor.expose_for_adapter()).ok_or_else(invalid)?);
        }
        let listed = builder
            .execute()
            .into_send()
            .await
            .map_err(map_worker_error)?;
        let mut items = Vec::new();
        for object in listed.objects() {
            let key = ScopedObjectKey::parse(&object.key()).map_err(|_| integrity())?;
            if !key.belongs_to(request.tenant_id(), request.video_id())
                || request.role().is_some_and(|role| key.role() != role)
            {
                return Err(integrity());
            }
            items.push(Self::metadata(&object, &key)?);
        }
        let next_cursor = listed
            .cursor()
            .map(|cursor| StorageListCursor::parse(encode_cursor(&cursor)))
            .transpose()?;
        Ok(ListObjectsPageV1 { items, next_cursor })
    }
}

fn cache_policy_name(policy: ObjectCachePolicy) -> &'static str {
    match policy {
        ObjectCachePolicy::NoStore => "no_store",
        ObjectCachePolicy::PrivateImmutable => "private_immutable",
        ObjectCachePolicy::PublicImmutable => "public_immutable",
    }
}

fn parse_cache_policy(value: &str) -> Option<ObjectCachePolicy> {
    match value {
        "no_store" => Some(ObjectCachePolicy::NoStore),
        "private_immutable" => Some(ObjectCachePolicy::PrivateImmutable),
        "public_immutable" => Some(ObjectCachePolicy::PublicImmutable),
        _ => None,
    }
}

fn cache_control(policy: ObjectCachePolicy) -> &'static str {
    match policy {
        ObjectCachePolicy::NoStore => "private, no-store",
        ObjectCachePolicy::PrivateImmutable => "private, max-age=31536000, immutable",
        ObjectCachePolicy::PublicImmutable => "public, max-age=31536000, immutable",
    }
}

fn map_worker_error(error: worker::Error) -> StorageFailure {
    let message = error.to_string().to_ascii_lowercase();
    let kind = if message.contains("precondition") || message.contains("412") {
        StorageFailureKind::PreconditionFailed
    } else if message.contains("unauthorized")
        || message.contains("forbidden")
        || message.contains("401")
        || message.contains("403")
    {
        StorageFailureKind::Unauthorized
    } else if message.contains("quota") {
        StorageFailureKind::QuotaExceeded
    } else if message.contains("timeout") {
        StorageFailureKind::Timeout
    } else if message.contains("rate") || message.contains("429") {
        StorageFailureKind::Throttled
    } else {
        StorageFailureKind::Unavailable
    };
    StorageFailure::new(kind)
}

fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(output)
}

fn nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        output.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    output
}

fn encode_cursor(value: &str) -> String {
    hex(value.as_bytes())
}

fn decode_cursor(value: &str) -> Option<String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        bytes.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    String::from_utf8(bytes).ok()
}

fn require_failure_kind<T>(
    result: Result<T, StorageFailure>,
    expected: StorageFailureKind,
) -> Result<(), StorageFailure> {
    match result {
        Err(error) if error.kind() == expected => Ok(()),
        _ => Err(integrity()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_codec_round_trips_opaque_provider_tokens() {
        for value in ["opaque+/=token", "abc.123_-", "provider-cursor"] {
            let encoded = encode_cursor(value);
            assert!(encoded.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(decode_cursor(&encoded).as_deref(), Some(value));
        }
        assert!(decode_cursor("abc").is_none());
        assert!(decode_cursor("zz").is_none());
    }

    #[test]
    fn cache_contract_round_trips() {
        for policy in [
            ObjectCachePolicy::NoStore,
            ObjectCachePolicy::PrivateImmutable,
            ObjectCachePolicy::PublicImmutable,
        ] {
            assert_eq!(parse_cache_policy(cache_policy_name(policy)), Some(policy));
            assert!(cache_control(policy).contains(
                if policy == ObjectCachePolicy::PublicImmutable {
                    "public"
                } else {
                    "private"
                }
            ));
        }
    }

    #[test]
    fn provider_errors_map_to_safe_taxonomy() {
        let failure = map_worker_error(worker::Error::RustError("HTTP 412 precondition".into()));
        assert_eq!(failure.kind(), StorageFailureKind::PreconditionFailed);
        assert!(!failure.to_string().contains("412"));
        let failure = map_worker_error(worker::Error::RustError("HTTP 429 throttled".into()));
        assert_eq!(failure.kind(), StorageFailureKind::Throttled);
    }

    #[test]
    fn tenant_fence_is_opaque_before_r2_access() {
        let owner = TenantId::new();
        let key = ScopedObjectKey::source(
            owner,
            VideoId::new(),
            ObjectRevision::new(1).expect("revision"),
            VideoObjectDescriptor::Source {
                extension: StorageFileExtension::parse("bin").expect("extension"),
            },
        )
        .expect("key");
        assert_eq!(
            R2ObjectStoreV1::authorize(
                StorageRequestContext::new(TenantId::new(), CorrelationId::new()),
                &key,
            )
            .expect_err("cross-tenant access must fail")
            .kind(),
            StorageFailureKind::NotFound
        );
        assert!(
            R2ObjectStoreV1::authorize(
                StorageRequestContext::new(owner, CorrelationId::new()),
                &key,
            )
            .is_ok()
        );
    }
}
