use std::{
    collections::{HashMap, HashSet},
    fmt,
    future::Future,
    num::{NonZeroU8, NonZeroUsize},
    pin::Pin,
    sync::{
        Mutex, RwLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use async_trait::async_trait;
use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, ObjectKey, ObjectRole, TenantId, VideoId,
};
use frame_ports::PortError;
use futures::{StreamExt, stream};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const OBJECT_BACKFILL_MANIFEST_VERSION: u16 = 1;

/// A provider checksum is opaque because multipart etags are not necessarily content hashes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProviderChecksum(String);

impl ProviderChecksum {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(BackfillError::InvalidManifest);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderChecksum {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderChecksum([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageProvider {
    S3,
    R2,
    Minio,
    GoogleDrive,
    Custom,
}

impl StorageProvider {
    const fn tag(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::R2 => "r2",
            Self::Minio => "minio",
            Self::GoogleDrive => "google-drive",
            Self::Custom => "custom",
        }
    }
}

/// Stable, credential-free identity for one storage authority.
#[derive(Clone, PartialEq, Eq)]
pub struct StorageIdentity {
    provider: StorageProvider,
    region: String,
    authority_fingerprint: ChecksumSha256,
}

impl StorageIdentity {
    pub fn new(
        provider: StorageProvider,
        region: impl Into<String>,
        authority_fingerprint: ChecksumSha256,
    ) -> Result<Self, BackfillError> {
        let region = region.into();
        if region.is_empty()
            || region.len() > 64
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(BackfillError::InvalidManifest);
        }
        Ok(Self {
            provider,
            region,
            authority_fingerprint,
        })
    }

    #[must_use]
    pub const fn provider(&self) -> StorageProvider {
        self.provider
    }

    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }

    #[must_use]
    pub fn authority_fingerprint(&self) -> &ChecksumSha256 {
        &self.authority_fingerprint
    }
}

impl fmt::Debug for StorageIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageIdentity")
            .field("provider", &self.provider)
            .field("region", &"[redacted]")
            .field("authority_fingerprint", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IntegrityExpectation {
    sha256: Option<ChecksumSha256>,
    provider_checksum: Option<ProviderChecksum>,
    media_probe_required: bool,
}

impl IntegrityExpectation {
    pub fn new(
        sha256: Option<ChecksumSha256>,
        provider_checksum: Option<ProviderChecksum>,
        media_probe_required: bool,
    ) -> Result<Self, BackfillError> {
        // Cross-provider migration cannot treat multipart etags as content hashes.
        if sha256.is_none() {
            return Err(BackfillError::InvalidManifest);
        }
        Ok(Self {
            sha256,
            provider_checksum,
            media_probe_required,
        })
    }

    #[must_use]
    pub const fn media_probe_required(&self) -> bool {
        self.media_probe_required
    }

    #[must_use]
    pub fn sha256(&self) -> &ChecksumSha256 {
        self.sha256
            .as_ref()
            .expect("integrity construction requires SHA-256")
    }

    #[must_use]
    pub fn provider_checksum(&self) -> Option<&ProviderChecksum> {
        self.provider_checksum.as_ref()
    }
}

impl fmt::Debug for IntegrityExpectation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntegrityExpectation")
            .field("sha256", &self.sha256.as_ref().map(|_| "[redacted]"))
            .field(
                "provider_checksum",
                &self.provider_checksum.as_ref().map(|_| "[redacted]"),
            )
            .field("media_probe_required", &self.media_probe_required)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    tenant_id: TenantId,
    video_id: VideoId,
    role: ObjectRole,
    source_key: ObjectKey,
    target_key: ObjectKey,
    expected_size: ByteSize,
    integrity: IntegrityExpectation,
}

impl ManifestEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        source_key: ObjectKey,
        target_key: ObjectKey,
        expected_size: ByteSize,
        integrity: IntegrityExpectation,
    ) -> Result<Self, BackfillError> {
        if expected_size.get() == 0
            || !source_key.belongs_to_tenant(tenant_id)
            || !target_key.belongs_to_tenant(tenant_id)
        {
            return Err(BackfillError::InvalidManifest);
        }
        Ok(Self {
            tenant_id,
            video_id,
            role,
            source_key,
            target_key,
            expected_size,
            integrity,
        })
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn video_id(&self) -> VideoId {
        self.video_id
    }

    #[must_use]
    pub const fn role(&self) -> ObjectRole {
        self.role
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub fn source_key(&self) -> &ObjectKey {
        &self.source_key
    }

    #[must_use]
    pub fn target_key(&self) -> &ObjectKey {
        &self.target_key
    }

    #[must_use]
    pub const fn integrity(&self) -> &IntegrityExpectation {
        &self.integrity
    }
}

impl fmt::Debug for ManifestEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManifestEntry")
            .field("tenant_id", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("role", &self.role)
            .field("source_key", &"[redacted]")
            .field("target_key", &"[redacted]")
            .field("expected_size", &self.expected_size)
            .field("integrity", &self.integrity)
            .finish()
    }
}

/// An immutable input manifest. Mutable attempts and statuses live in `BackfillCheckpoint`.
#[derive(Clone, PartialEq, Eq)]
pub struct BackfillManifest {
    schema_version: u16,
    tool_version: String,
    source: StorageIdentity,
    target: StorageIdentity,
    entries: Vec<ManifestEntry>,
    digest: ChecksumSha256,
}

impl BackfillManifest {
    pub fn new(
        tool_version: impl Into<String>,
        source: StorageIdentity,
        target: StorageIdentity,
        entries: Vec<ManifestEntry>,
    ) -> Result<Self, BackfillError> {
        let tool_version = tool_version.into();
        if tool_version.is_empty()
            || tool_version.len() > 64
            || !tool_version.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+')
            })
            || entries.is_empty()
        {
            return Err(BackfillError::InvalidManifest);
        }
        let mut targets = HashSet::with_capacity(entries.len());
        if entries
            .iter()
            .any(|entry| !targets.insert(entry.target_key.as_str().to_owned()))
        {
            return Err(BackfillError::InvalidManifest);
        }
        let digest = manifest_digest(&tool_version, &source, &target, &entries)?;
        Ok(Self {
            schema_version: OBJECT_BACKFILL_MANIFEST_VERSION,
            tool_version,
            source,
            target,
            entries,
            digest,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub fn digest(&self) -> &ChecksumSha256 {
        &self.digest
    }

    #[must_use]
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    #[must_use]
    pub fn tool_version(&self) -> &str {
        &self.tool_version
    }

    #[must_use]
    pub const fn source_identity(&self) -> &StorageIdentity {
        &self.source
    }

    #[must_use]
    pub const fn target_identity(&self) -> &StorageIdentity {
        &self.target
    }
}

impl fmt::Debug for BackfillManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillManifest")
            .field("schema_version", &self.schema_version)
            .field("tool_version", &self.tool_version)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("entry_count", &self.entries.len())
            .field("digest", &"[redacted]")
            .finish()
    }
}

fn manifest_digest(
    tool_version: &str,
    source: &StorageIdentity,
    target: &StorageIdentity,
    entries: &[ManifestEntry],
) -> Result<ChecksumSha256, BackfillError> {
    let mut hasher = Sha256::new();
    update_digest_field(&mut hasher, &OBJECT_BACKFILL_MANIFEST_VERSION.to_be_bytes());
    update_digest_field(&mut hasher, tool_version.as_bytes());
    update_identity_digest(&mut hasher, source);
    update_identity_digest(&mut hasher, target);
    for entry in entries {
        update_digest_field(&mut hasher, entry.tenant_id.to_string().as_bytes());
        update_digest_field(&mut hasher, entry.video_id.to_string().as_bytes());
        update_digest_field(&mut hasher, entry.role.path_segment().as_bytes());
        update_digest_field(&mut hasher, entry.source_key.as_str().as_bytes());
        update_digest_field(&mut hasher, entry.target_key.as_str().as_bytes());
        update_digest_field(&mut hasher, &entry.expected_size.get().to_be_bytes());
        if let Some(checksum) = &entry.integrity.sha256 {
            update_digest_field(&mut hasher, checksum.as_str().as_bytes());
        }
        if let Some(checksum) = &entry.integrity.provider_checksum {
            update_digest_field(&mut hasher, checksum.as_str().as_bytes());
        }
        update_digest_field(
            &mut hasher,
            &[u8::from(entry.integrity.media_probe_required)],
        );
    }
    ChecksumSha256::parse(hex_digest(hasher.finalize().as_slice()))
        .map_err(|_| BackfillError::InvalidManifest)
}

fn update_identity_digest(hasher: &mut Sha256, identity: &StorageIdentity) {
    update_digest_field(hasher, identity.provider.tag().as_bytes());
    update_digest_field(hasher, identity.region.as_bytes());
    update_digest_field(hasher, identity.authority_fingerprint.as_str().as_bytes());
}

fn update_digest_field(hasher: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn sha256(bytes: &[u8]) -> Result<ChecksumSha256, BackfillError> {
    ChecksumSha256::parse(hex_digest(Sha256::digest(bytes).as_slice()))
        .map_err(|_| BackfillError::InvalidManifest)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BackfillScope(TenantId);

impl BackfillScope {
    #[must_use]
    pub const fn tenant(tenant_id: TenantId) -> Self {
        Self(tenant_id)
    }
}

impl fmt::Debug for BackfillScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillScope::Tenant([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillPolicy {
    max_concurrency: NonZeroUsize,
    max_attempts: NonZeroU8,
    max_entries_per_run: NonZeroUsize,
    max_logical_bytes_per_run: u64,
}

impl BackfillPolicy {
    pub fn new(
        max_concurrency: usize,
        max_attempts: u8,
        max_entries_per_run: usize,
        max_logical_bytes_per_run: u64,
    ) -> Result<Self, BackfillError> {
        Ok(Self {
            max_concurrency: NonZeroUsize::new(max_concurrency)
                .ok_or(BackfillError::InvalidPolicy)?,
            max_attempts: NonZeroU8::new(max_attempts).ok_or(BackfillError::InvalidPolicy)?,
            max_entries_per_run: NonZeroUsize::new(max_entries_per_run)
                .ok_or(BackfillError::InvalidPolicy)?,
            max_logical_bytes_per_run: (max_logical_bytes_per_run > 0)
                .then_some(max_logical_bytes_per_run)
                .ok_or(BackfillError::InvalidPolicy)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineReason {
    MissingSource,
    SourceSizeMismatch,
    SourceChecksumMismatch,
    SourceProbeFailed,
    DestinationConflict,
    OwnershipMismatch,
    ProviderUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineDisposition {
    PendingOwnerDecision,
    RetryApproved,
    ReferenceApproved,
    ExcludeApproved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillStatus {
    Pending,
    Copied,
    Reused,
    Quarantined {
        reason: QuarantineReason,
        disposition: QuarantineDisposition,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointEntry {
    status: BackfillStatus,
    attempts: u32,
}

impl CheckpointEntry {
    #[must_use]
    pub const fn new(status: BackfillStatus, attempts: u32) -> Self {
        Self { status, attempts }
    }

    #[must_use]
    pub const fn status(self) -> BackfillStatus {
        self.status
    }

    #[must_use]
    pub const fn attempts(self) -> u32 {
        self.attempts
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillCheckpoint {
    manifest_digest: ChecksumSha256,
    entries: Vec<CheckpointEntry>,
    revision: u64,
}

impl BackfillCheckpoint {
    #[must_use]
    pub fn new(manifest: &BackfillManifest) -> Self {
        Self {
            manifest_digest: manifest.digest.clone(),
            entries: vec![
                CheckpointEntry {
                    status: BackfillStatus::Pending,
                    attempts: 0,
                };
                manifest.entries.len()
            ],
            revision: 0,
        }
    }

    #[must_use]
    pub fn status(&self, ordinal: usize) -> Option<BackfillStatus> {
        self.entries.get(ordinal).map(|entry| entry.status)
    }

    #[must_use]
    pub fn attempts(&self, ordinal: usize) -> Option<u32> {
        self.entries.get(ordinal).map(|entry| entry.attempts)
    }

    #[must_use]
    pub fn entries(&self) -> &[CheckpointEntry] {
        &self.entries
    }

    pub fn restore(
        manifest: &BackfillManifest,
        persisted_manifest_digest: ChecksumSha256,
        entries: Vec<CheckpointEntry>,
        revision: u64,
    ) -> Result<Self, BackfillError> {
        if persisted_manifest_digest != manifest.digest || entries.len() != manifest.entries.len() {
            return Err(BackfillError::CheckpointMismatch);
        }
        let invalid_progress = entries.iter().any(|entry| {
            matches!(
                entry.status,
                BackfillStatus::Copied
                    | BackfillStatus::Reused
                    | BackfillStatus::Quarantined { .. }
            ) && entry.attempts == 0
        });
        if invalid_progress {
            return Err(BackfillError::CheckpointMismatch);
        }
        Ok(Self {
            manifest_digest: persisted_manifest_digest,
            entries,
            revision,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn approve_quarantine(
        &mut self,
        ordinal: usize,
        disposition: QuarantineDisposition,
    ) -> Result<(), BackfillError> {
        if disposition == QuarantineDisposition::PendingOwnerDecision {
            return Err(BackfillError::InvalidDisposition);
        }
        let progress = self
            .entries
            .get_mut(ordinal)
            .ok_or(BackfillError::StateConflict)?;
        let BackfillStatus::Quarantined { reason, .. } = progress.status else {
            return Err(BackfillError::StateConflict);
        };
        progress.status = BackfillStatus::Quarantined {
            reason,
            disposition,
        };
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    fn eligible(&self, ordinal: usize) -> bool {
        self.entries.get(ordinal).is_some_and(|entry| {
            matches!(entry.status, BackfillStatus::Pending)
                || matches!(
                    entry.status,
                    BackfillStatus::Quarantined {
                        disposition: QuarantineDisposition::RetryApproved,
                        ..
                    }
                )
        })
    }
}

impl fmt::Debug for BackfillCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillCheckpoint")
            .field("manifest_digest", &"[redacted]")
            .field("entry_count", &self.entries.len())
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ObjectDescriptor {
    owner_tenant: TenantId,
    key: ObjectKey,
    size: ByteSize,
    content_type: ContentType,
    provider_checksum: Option<ProviderChecksum>,
}

impl ObjectDescriptor {
    #[must_use]
    pub const fn owner_tenant(&self) -> TenantId {
        self.owner_tenant
    }

    #[must_use]
    pub const fn size(&self) -> ByteSize {
        self.size
    }
}

impl fmt::Debug for ObjectDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectDescriptor")
            .field("owner_tenant", &"[redacted]")
            .field("key", &"[redacted]")
            .field("size", &self.size)
            .field("content_type", &self.content_type)
            .field(
                "provider_checksum",
                &self.provider_checksum.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillObject {
    descriptor: ObjectDescriptor,
    bytes: Vec<u8>,
}

impl BackfillObject {
    #[must_use]
    pub fn descriptor(&self) -> &ObjectDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for BackfillObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillObject")
            .field("descriptor", &self.descriptor)
            .field(
                "bytes",
                &format_args!("[redacted; {} bytes]", self.bytes.len()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Created,
    AlreadyExists,
}

#[async_trait]
pub trait BackfillObjectStore: Send + Sync {
    async fn read(&self, key: &ObjectKey) -> Result<Option<BackfillObject>, PortError>;

    async fn put_if_absent(
        &self,
        owner_tenant: TenantId,
        key: &ObjectKey,
        bytes: Vec<u8>,
        content_type: ContentType,
    ) -> Result<PutOutcome, PortError>;

    async fn list_for_tenant(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<ObjectDescriptor>, PortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageOperation {
    Read,
    PutBeforeCommit,
    PutAfterCommit,
    List,
}

#[derive(Clone)]
struct MemoryObject {
    owner_tenant: TenantId,
    key: ObjectKey,
    bytes: Vec<u8>,
    content_type: ContentType,
    provider_checksum: Option<ProviderChecksum>,
}

#[derive(Default)]
pub struct MemoryBackfillObjectStore {
    objects: RwLock<HashMap<String, MemoryObject>>,
    failures: Mutex<HashMap<StorageOperation, usize>>,
    successful_writes: AtomicU64,
    active_operations: AtomicUsize,
    max_active_operations: AtomicUsize,
}

impl MemoryBackfillObjectStore {
    pub fn seed(
        &self,
        owner_tenant: TenantId,
        key: ObjectKey,
        bytes: Vec<u8>,
        content_type: ContentType,
        provider_checksum: Option<ProviderChecksum>,
    ) -> Result<(), BackfillError> {
        if bytes.is_empty() {
            return Err(BackfillError::InvalidManifest);
        }
        self.objects
            .write()
            .map_err(|_| BackfillError::StateConflict)?
            .insert(
                key.as_str().to_owned(),
                MemoryObject {
                    owner_tenant,
                    key,
                    bytes,
                    content_type,
                    provider_checksum,
                },
            );
        Ok(())
    }

    pub fn fail_next(
        &self,
        operation: StorageOperation,
        count: usize,
    ) -> Result<(), BackfillError> {
        self.failures
            .lock()
            .map_err(|_| BackfillError::StateConflict)?
            .insert(operation, count);
        Ok(())
    }

    #[must_use]
    pub fn successful_writes(&self) -> u64 {
        self.successful_writes.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn max_active_operations(&self) -> usize {
        self.max_active_operations.load(Ordering::Relaxed)
    }

    fn maybe_fail(&self, operation: StorageOperation) -> Result<(), PortError> {
        let mut failures = self
            .failures
            .lock()
            .map_err(|_| PortError::Adapter("in-memory failure lock poisoned".into()))?;
        let Some(remaining) = failures.get_mut(&operation) else {
            return Ok(());
        };
        if *remaining == 0 {
            return Ok(());
        }
        *remaining -= 1;
        Err(PortError::Adapter(
            "provider detail https://private.example/?token=secret".into(),
        ))
    }

    fn operation_guard(&self) -> OperationGuard<'_> {
        let active = self.active_operations.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active_operations
            .fetch_max(active, Ordering::SeqCst);
        OperationGuard {
            active: &self.active_operations,
        }
    }

    fn descriptor(object: &MemoryObject) -> Result<ObjectDescriptor, PortError> {
        let size = u64::try_from(object.bytes.len())
            .ok()
            .and_then(|value| ByteSize::new(value).ok())
            .ok_or_else(|| PortError::Adapter("in-memory object is too large".into()))?;
        Ok(ObjectDescriptor {
            owner_tenant: object.owner_tenant,
            key: object.key.clone(),
            size,
            content_type: object.content_type.clone(),
            provider_checksum: object.provider_checksum.clone(),
        })
    }
}

impl fmt::Debug for MemoryBackfillObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryBackfillObjectStore")
            .field(
                "object_count",
                &self.objects.read().map_or(0, |objects| objects.len()),
            )
            .field("successful_writes", &self.successful_writes())
            .finish()
    }
}

struct OperationGuard<'a> {
    active: &'a AtomicUsize,
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[async_trait]
impl BackfillObjectStore for MemoryBackfillObjectStore {
    async fn read(&self, key: &ObjectKey) -> Result<Option<BackfillObject>, PortError> {
        let _guard = self.operation_guard();
        YieldOnce(false).await;
        self.maybe_fail(StorageOperation::Read)?;
        let objects = self
            .objects
            .read()
            .map_err(|_| PortError::Adapter("in-memory object lock poisoned".into()))?;
        objects
            .get(key.as_str())
            .map(|object| {
                Ok(BackfillObject {
                    descriptor: Self::descriptor(object)?,
                    bytes: object.bytes.clone(),
                })
            })
            .transpose()
    }

    async fn put_if_absent(
        &self,
        owner_tenant: TenantId,
        key: &ObjectKey,
        bytes: Vec<u8>,
        content_type: ContentType,
    ) -> Result<PutOutcome, PortError> {
        let _guard = self.operation_guard();
        YieldOnce(false).await;
        self.maybe_fail(StorageOperation::PutBeforeCommit)?;
        if bytes.is_empty() || !key.belongs_to_tenant(owner_tenant) {
            return Err(PortError::InvalidRequest(
                "object does not belong to the requested tenant".into(),
            ));
        }
        let mut objects = self
            .objects
            .write()
            .map_err(|_| PortError::Adapter("in-memory object lock poisoned".into()))?;
        if objects.contains_key(key.as_str()) {
            return Ok(PutOutcome::AlreadyExists);
        }
        objects.insert(
            key.as_str().to_owned(),
            MemoryObject {
                owner_tenant,
                key: key.clone(),
                bytes,
                content_type,
                provider_checksum: None,
            },
        );
        self.successful_writes.fetch_add(1, Ordering::Relaxed);
        drop(objects);
        self.maybe_fail(StorageOperation::PutAfterCommit)?;
        Ok(PutOutcome::Created)
    }

    async fn list_for_tenant(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<ObjectDescriptor>, PortError> {
        let _guard = self.operation_guard();
        YieldOnce(false).await;
        self.maybe_fail(StorageOperation::List)?;
        self.objects
            .read()
            .map_err(|_| PortError::Adapter("in-memory object lock poisoned".into()))?
            .values()
            .filter(|object| object.owner_tenant == tenant_id)
            .map(Self::descriptor)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ProbeError {
    #[error("media probe rejected the object")]
    InvalidMedia,
}

pub trait ObjectProbe: Send + Sync {
    fn probe(&self, role: ObjectRole, bytes: &[u8]) -> Result<(), ProbeError>;
}

/// A cheap preflight probe. Production adapters can replace this with ffprobe/GStreamer.
#[derive(Debug, Default, Clone, Copy)]
pub struct BasicMediaProbe;

impl ObjectProbe for BasicMediaProbe {
    fn probe(&self, _role: ObjectRole, bytes: &[u8]) -> Result<(), ProbeError> {
        if bytes.is_empty() || bytes.starts_with(b"CORRUPT") || bytes.iter().all(|byte| *byte == 0)
        {
            Err(ProbeError::InvalidMedia)
        } else {
            Ok(())
        }
    }
}

pub struct BackfillCoordinator<'a> {
    source: &'a dyn BackfillObjectStore,
    target: &'a dyn BackfillObjectStore,
    probe: &'a dyn ObjectProbe,
    source_identity: StorageIdentity,
    target_identity: StorageIdentity,
}

impl<'a> BackfillCoordinator<'a> {
    #[must_use]
    pub fn new(
        source: &'a dyn BackfillObjectStore,
        target: &'a dyn BackfillObjectStore,
        probe: &'a dyn ObjectProbe,
        source_identity: StorageIdentity,
        target_identity: StorageIdentity,
    ) -> Self {
        Self {
            source,
            target,
            probe,
            source_identity,
            target_identity,
        }
    }

    pub async fn run(
        &self,
        scope: BackfillScope,
        manifest: &BackfillManifest,
        checkpoint: &mut BackfillCheckpoint,
        policy: BackfillPolicy,
    ) -> Result<BackfillRunReport, BackfillError> {
        self.validate_context(manifest, checkpoint)?;

        let eligible = manifest
            .entries
            .iter()
            .enumerate()
            .filter(|(ordinal, entry)| entry.tenant_id == scope.0 && checkpoint.eligible(*ordinal))
            .collect::<Vec<_>>();
        let mut selected = Vec::new();
        let mut selected_bytes = 0_u64;
        for (ordinal, entry) in &eligible {
            if entry.expected_size.get() > policy.max_logical_bytes_per_run {
                return Err(BackfillError::InvalidPolicy);
            }
            if selected.len() >= policy.max_entries_per_run.get() {
                break;
            }
            let next_bytes = selected_bytes.saturating_add(entry.expected_size.get());
            if !selected.is_empty() && next_bytes > policy.max_logical_bytes_per_run {
                break;
            }
            selected.push((*ordinal, *entry));
            selected_bytes = next_bytes;
        }

        let mut executions =
            stream::iter(selected.into_iter().map(|(ordinal, entry)| async move {
                (ordinal, self.execute_with_retry(entry, policy).await)
            }))
            .buffer_unordered(policy.max_concurrency.get())
            .collect::<Vec<_>>()
            .await;
        executions.sort_by_key(|(ordinal, _)| *ordinal);

        let mut copied = 0_usize;
        let mut reused = 0_usize;
        let mut quarantined = 0_usize;
        let mut retries = 0_u32;
        for (ordinal, execution) in executions {
            let progress = checkpoint
                .entries
                .get_mut(ordinal)
                .ok_or(BackfillError::CheckpointMismatch)?;
            progress.attempts = progress
                .attempts
                .saturating_add(u32::from(execution.attempts));
            retries = retries.saturating_add(u32::from(execution.attempts.saturating_sub(1)));
            progress.status = match execution.outcome {
                Ok(ExecutionKind::Copied) => {
                    copied += 1;
                    BackfillStatus::Copied
                }
                Ok(ExecutionKind::Reused) => {
                    reused += 1;
                    BackfillStatus::Reused
                }
                Err(reason) => {
                    quarantined += 1;
                    BackfillStatus::Quarantined {
                        reason,
                        disposition: QuarantineDisposition::PendingOwnerDecision,
                    }
                }
            };
            checkpoint.revision = checkpoint.revision.saturating_add(1);
        }

        let remaining = manifest
            .entries
            .iter()
            .enumerate()
            .filter(|(ordinal, entry)| entry.tenant_id == scope.0 && checkpoint.eligible(*ordinal))
            .count();
        Ok(BackfillRunReport {
            attempted: copied + reused + quarantined,
            copied,
            reused,
            quarantined,
            retries,
            remaining,
            interrupted: remaining > 0 || eligible.len() > copied + reused + quarantined,
            logical_bytes_selected: selected_bytes,
        })
    }

    async fn execute_with_retry(
        &self,
        entry: &ManifestEntry,
        policy: BackfillPolicy,
    ) -> EntryExecution {
        let mut attempts = 0_u8;
        loop {
            attempts = attempts.saturating_add(1);
            match self.copy_once(entry).await {
                Ok(outcome) => {
                    return EntryExecution {
                        attempts,
                        outcome: Ok(outcome),
                    };
                }
                Err(failure) if failure.retryable && attempts < policy.max_attempts.get() => {}
                Err(failure) => {
                    return EntryExecution {
                        attempts,
                        outcome: Err(failure.reason),
                    };
                }
            }
        }
    }

    async fn copy_once(&self, entry: &ManifestEntry) -> Result<ExecutionKind, CopyFailure> {
        if let Some(existing) = self
            .target
            .read(&entry.target_key)
            .await
            .map_err(|error| store_failure(error, ObjectSide::Target))?
        {
            self.verify_object(entry, &existing, ObjectSide::Target)?;
            return Ok(ExecutionKind::Reused);
        }

        let source = self
            .source
            .read(&entry.source_key)
            .await
            .map_err(|error| store_failure(error, ObjectSide::Source))?
            .ok_or(CopyFailure::terminal(QuarantineReason::MissingSource))?;
        self.verify_object(entry, &source, ObjectSide::Source)?;

        let outcome = self
            .target
            .put_if_absent(
                entry.tenant_id,
                &entry.target_key,
                source.bytes.clone(),
                source.descriptor.content_type.clone(),
            )
            .await
            .map_err(|error| store_failure(error, ObjectSide::Target))?;
        match outcome {
            PutOutcome::Created => Ok(ExecutionKind::Copied),
            PutOutcome::AlreadyExists => {
                let existing = self
                    .target
                    .read(&entry.target_key)
                    .await
                    .map_err(|error| store_failure(error, ObjectSide::Target))?
                    .ok_or(CopyFailure::terminal(QuarantineReason::DestinationConflict))?;
                self.verify_object(entry, &existing, ObjectSide::Target)?;
                Ok(ExecutionKind::Reused)
            }
        }
    }

    fn verify_object(
        &self,
        entry: &ManifestEntry,
        object: &BackfillObject,
        side: ObjectSide,
    ) -> Result<(), CopyFailure> {
        if object.descriptor.owner_tenant != entry.tenant_id
            || !object.descriptor.key.belongs_to_tenant(entry.tenant_id)
        {
            return Err(CopyFailure::terminal(QuarantineReason::OwnershipMismatch));
        }
        if object.descriptor.size != entry.expected_size {
            return Err(CopyFailure::terminal(side.size_reason()));
        }
        if matches!(side, ObjectSide::Source)
            && let Some(expected) = &entry.integrity.provider_checksum
            && object.descriptor.provider_checksum.as_ref() != Some(expected)
        {
            return Err(CopyFailure::terminal(side.checksum_reason()));
        }
        if let Some(expected) = &entry.integrity.sha256 {
            let actual =
                sha256(&object.bytes).map_err(|_| CopyFailure::terminal(side.checksum_reason()))?;
            if &actual != expected {
                return Err(CopyFailure::terminal(side.checksum_reason()));
            }
        }
        if entry.integrity.media_probe_required
            && self.probe.probe(entry.role, &object.bytes).is_err()
        {
            return Err(CopyFailure::terminal(side.probe_reason()));
        }
        Ok(())
    }

    fn validate_context(
        &self,
        manifest: &BackfillManifest,
        checkpoint: &BackfillCheckpoint,
    ) -> Result<(), BackfillError> {
        if manifest.source != self.source_identity || manifest.target != self.target_identity {
            return Err(BackfillError::IdentityMismatch);
        }
        if manifest.digest != checkpoint.manifest_digest
            || manifest.entries.len() != checkpoint.entries.len()
        {
            return Err(BackfillError::CheckpointMismatch);
        }
        Ok(())
    }

    pub async fn reconcile(
        &self,
        scope: BackfillScope,
        manifest: &BackfillManifest,
        checkpoint: &BackfillCheckpoint,
    ) -> Result<ReconciliationReport, BackfillError> {
        self.validate_context(manifest, checkpoint)?;
        let mut report = ReconciliationReport::default();
        let scoped = manifest
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.tenant_id == scope.0)
            .collect::<Vec<_>>();
        let mut source_keys = HashMap::<String, usize>::new();
        let target_keys = scoped
            .iter()
            .map(|(_, entry)| entry.target_key.as_str().to_owned())
            .collect::<HashSet<_>>();

        for (ordinal, entry) in scoped {
            report.expected_objects += 1;
            report.expected_logical_bytes = report
                .expected_logical_bytes
                .saturating_add(entry.expected_size.get());
            *report.expected_role_counts.entry(entry.role).or_default() += 1;
            if source_keys
                .insert(entry.source_key.as_str().to_owned(), ordinal)
                .is_some()
            {
                report.push(ordinal, entry.role, DiscrepancyKind::DuplicateSource);
            }

            match self.source.read(&entry.source_key).await {
                Ok(Some(object)) => {
                    report.observed_source_objects += 1;
                    *report
                        .observed_source_role_counts
                        .entry(entry.role)
                        .or_default() += 1;
                    report.observed_source_bytes = report
                        .observed_source_bytes
                        .saturating_add(object.descriptor.size.get());
                    match self.verify_object(entry, &object, ObjectSide::Source) {
                        Ok(()) => report.verified_checksums += 1,
                        Err(failure) => report.push(
                            ordinal,
                            entry.role,
                            discrepancy_from_quarantine(failure.reason, ObjectSide::Source),
                        ),
                    }
                }
                Ok(None) => report.push(ordinal, entry.role, DiscrepancyKind::MissingSource),
                Err(_) => report.push(ordinal, entry.role, DiscrepancyKind::ProviderUnavailable),
            }

            match self.target.read(&entry.target_key).await {
                Ok(Some(object)) => {
                    report.observed_target_objects += 1;
                    *report
                        .observed_target_role_counts
                        .entry(entry.role)
                        .or_default() += 1;
                    report.observed_target_bytes = report
                        .observed_target_bytes
                        .saturating_add(object.descriptor.size.get());
                    match self.verify_object(entry, &object, ObjectSide::Target) {
                        Ok(()) => report.verified_checksums += 1,
                        Err(failure) => {
                            report.push(
                                ordinal,
                                entry.role,
                                discrepancy_from_quarantine(failure.reason, ObjectSide::Target),
                            );
                            if matches!(
                                checkpoint.status(ordinal),
                                Some(BackfillStatus::Copied | BackfillStatus::Reused)
                            ) {
                                report.push(
                                    ordinal,
                                    entry.role,
                                    DiscrepancyKind::CheckpointMismatch,
                                );
                            }
                        }
                    }
                }
                Ok(None) => {
                    report.push(ordinal, entry.role, DiscrepancyKind::MissingTarget);
                    if matches!(
                        checkpoint.status(ordinal),
                        Some(BackfillStatus::Copied | BackfillStatus::Reused)
                    ) {
                        report.push(ordinal, entry.role, DiscrepancyKind::CheckpointMismatch);
                    }
                }
                Err(_) => report.push(ordinal, entry.role, DiscrepancyKind::ProviderUnavailable),
            }
        }

        let listed = self
            .target
            .list_for_tenant(scope.0)
            .await
            .map_err(|_| BackfillError::InventoryUnavailable)?;
        for object in listed {
            if object.owner_tenant != scope.0 || !object.key.belongs_to_tenant(scope.0) {
                report.push_without_entry(DiscrepancyKind::OwnershipMismatch);
            } else if !target_keys.contains(object.key.as_str()) {
                report.push_without_entry(DiscrepancyKind::OrphanTarget);
            }
        }
        Ok(report)
    }
}

impl fmt::Debug for BackfillCoordinator<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillCoordinator")
            .field("source_identity", &self.source_identity)
            .field("target_identity", &self.target_identity)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionKind {
    Copied,
    Reused,
}

struct EntryExecution {
    attempts: u8,
    outcome: Result<ExecutionKind, QuarantineReason>,
}

#[derive(Debug, Clone, Copy)]
enum ObjectSide {
    Source,
    Target,
}

impl ObjectSide {
    const fn size_reason(self) -> QuarantineReason {
        match self {
            Self::Source => QuarantineReason::SourceSizeMismatch,
            Self::Target => QuarantineReason::DestinationConflict,
        }
    }

    const fn checksum_reason(self) -> QuarantineReason {
        match self {
            Self::Source => QuarantineReason::SourceChecksumMismatch,
            Self::Target => QuarantineReason::DestinationConflict,
        }
    }

    const fn probe_reason(self) -> QuarantineReason {
        match self {
            Self::Source => QuarantineReason::SourceProbeFailed,
            Self::Target => QuarantineReason::DestinationConflict,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CopyFailure {
    reason: QuarantineReason,
    retryable: bool,
}

impl CopyFailure {
    const fn terminal(reason: QuarantineReason) -> Self {
        Self {
            reason,
            retryable: false,
        }
    }
}

fn store_failure(error: PortError, side: ObjectSide) -> CopyFailure {
    match error {
        PortError::Adapter(_) => CopyFailure {
            reason: QuarantineReason::ProviderUnavailable,
            retryable: true,
        },
        PortError::NotFound if matches!(side, ObjectSide::Source) => {
            CopyFailure::terminal(QuarantineReason::MissingSource)
        }
        _ if matches!(side, ObjectSide::Source) => {
            CopyFailure::terminal(QuarantineReason::SourceChecksumMismatch)
        }
        _ => CopyFailure::terminal(QuarantineReason::DestinationConflict),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillRunReport {
    attempted: usize,
    copied: usize,
    reused: usize,
    quarantined: usize,
    retries: u32,
    remaining: usize,
    interrupted: bool,
    logical_bytes_selected: u64,
}

impl BackfillRunReport {
    #[must_use]
    pub const fn attempted(&self) -> usize {
        self.attempted
    }

    #[must_use]
    pub const fn copied(&self) -> usize {
        self.copied
    }

    #[must_use]
    pub const fn reused(&self) -> usize {
        self.reused
    }

    #[must_use]
    pub const fn quarantined(&self) -> usize {
        self.quarantined
    }

    #[must_use]
    pub const fn retries(&self) -> u32 {
        self.retries
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.remaining
    }

    #[must_use]
    pub const fn interrupted(&self) -> bool {
        self.interrupted
    }

    #[must_use]
    pub const fn logical_bytes_selected(&self) -> u64 {
        self.logical_bytes_selected
    }
}

impl fmt::Debug for BackfillRunReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillRunReport")
            .field("attempted", &self.attempted)
            .field("copied", &self.copied)
            .field("reused", &self.reused)
            .field("quarantined", &self.quarantined)
            .field("retries", &self.retries)
            .field("remaining", &self.remaining)
            .field("interrupted", &self.interrupted)
            .field("logical_bytes_selected", &self.logical_bytes_selected)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscrepancyKind {
    MissingSource,
    MissingTarget,
    DuplicateSource,
    OrphanTarget,
    OwnershipMismatch,
    CorruptSource,
    CorruptTarget,
    UnplayableSource,
    ProviderUnavailable,
    CheckpointMismatch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Discrepancy {
    ordinal: Option<usize>,
    role: Option<ObjectRole>,
    kind: DiscrepancyKind,
}

impl fmt::Debug for Discrepancy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Discrepancy")
            .field("ordinal", &self.ordinal)
            .field("role", &self.role)
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Default, Clone, PartialEq, Eq)]
pub struct ReconciliationReport {
    expected_objects: usize,
    observed_source_objects: usize,
    observed_target_objects: usize,
    expected_logical_bytes: u64,
    observed_source_bytes: u64,
    observed_target_bytes: u64,
    verified_checksums: usize,
    expected_role_counts: HashMap<ObjectRole, usize>,
    observed_source_role_counts: HashMap<ObjectRole, usize>,
    observed_target_role_counts: HashMap<ObjectRole, usize>,
    discrepancies: Vec<Discrepancy>,
}

impl ReconciliationReport {
    fn push(&mut self, ordinal: usize, role: ObjectRole, kind: DiscrepancyKind) {
        self.discrepancies.push(Discrepancy {
            ordinal: Some(ordinal),
            role: Some(role),
            kind,
        });
    }

    fn push_without_entry(&mut self, kind: DiscrepancyKind) {
        self.discrepancies.push(Discrepancy {
            ordinal: None,
            role: None,
            kind,
        });
    }

    #[must_use]
    pub fn clean(&self) -> bool {
        self.discrepancies.is_empty()
            && self.expected_objects == self.observed_source_objects
            && self.expected_objects == self.observed_target_objects
            && self.expected_logical_bytes == self.observed_source_bytes
            && self.expected_logical_bytes == self.observed_target_bytes
            && self.expected_role_counts == self.observed_source_role_counts
            && self.expected_role_counts == self.observed_target_role_counts
    }

    #[must_use]
    pub const fn expected_objects(&self) -> usize {
        self.expected_objects
    }

    #[must_use]
    pub const fn observed_source_objects(&self) -> usize {
        self.observed_source_objects
    }

    #[must_use]
    pub const fn observed_target_objects(&self) -> usize {
        self.observed_target_objects
    }

    #[must_use]
    pub const fn verified_checksums(&self) -> usize {
        self.verified_checksums
    }

    #[must_use]
    pub fn role_count(&self, role: ObjectRole) -> usize {
        self.expected_role_counts.get(&role).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn observed_source_role_count(&self, role: ObjectRole) -> usize {
        self.observed_source_role_counts
            .get(&role)
            .copied()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn observed_target_role_count(&self, role: ObjectRole) -> usize {
        self.observed_target_role_counts
            .get(&role)
            .copied()
            .unwrap_or(0)
    }

    pub fn discrepancy_kinds(&self) -> impl Iterator<Item = DiscrepancyKind> + '_ {
        self.discrepancies.iter().map(|item| item.kind)
    }

    #[must_use]
    pub fn repair_plan(&self) -> RepairPlan {
        RepairPlan {
            dry_run: true,
            actions: self
                .discrepancies
                .iter()
                .map(|discrepancy| match discrepancy.kind {
                    DiscrepancyKind::MissingTarget => {
                        RepairAction::CopyMissingTarget(discrepancy.ordinal)
                    }
                    DiscrepancyKind::MissingSource
                    | DiscrepancyKind::CorruptSource
                    | DiscrepancyKind::UnplayableSource => {
                        RepairAction::QuarantineSource(discrepancy.ordinal)
                    }
                    DiscrepancyKind::CorruptTarget | DiscrepancyKind::CheckpointMismatch => {
                        RepairAction::InvestigateConflict(discrepancy.ordinal)
                    }
                    DiscrepancyKind::DuplicateSource => {
                        RepairAction::ReviewDuplicate(discrepancy.ordinal)
                    }
                    DiscrepancyKind::OrphanTarget => RepairAction::ReviewOrphanTarget,
                    DiscrepancyKind::OwnershipMismatch => RepairAction::ReviewOwnership,
                    DiscrepancyKind::ProviderUnavailable => RepairAction::RetryInventory,
                })
                .collect(),
        }
    }
}

impl fmt::Debug for ReconciliationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReconciliationReport")
            .field("expected_objects", &self.expected_objects)
            .field("observed_source_objects", &self.observed_source_objects)
            .field("observed_target_objects", &self.observed_target_objects)
            .field("expected_logical_bytes", &self.expected_logical_bytes)
            .field("observed_source_bytes", &self.observed_source_bytes)
            .field("observed_target_bytes", &self.observed_target_bytes)
            .field("verified_checksums", &self.verified_checksums)
            .field("expected_role_counts", &self.expected_role_counts)
            .field(
                "observed_source_role_counts",
                &self.observed_source_role_counts,
            )
            .field(
                "observed_target_role_counts",
                &self.observed_target_role_counts,
            )
            .field("discrepancies", &self.discrepancies)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairAction {
    CopyMissingTarget(Option<usize>),
    QuarantineSource(Option<usize>),
    InvestigateConflict(Option<usize>),
    ReviewDuplicate(Option<usize>),
    ReviewOrphanTarget,
    ReviewOwnership,
    RetryInventory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairPlan {
    dry_run: bool,
    actions: Vec<RepairAction>,
}

impl RepairPlan {
    #[must_use]
    pub const fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    #[must_use]
    pub fn actions(&self) -> &[RepairAction] {
        &self.actions
    }
}

fn discrepancy_from_quarantine(reason: QuarantineReason, side: ObjectSide) -> DiscrepancyKind {
    match reason {
        QuarantineReason::MissingSource => DiscrepancyKind::MissingSource,
        QuarantineReason::SourceProbeFailed => DiscrepancyKind::UnplayableSource,
        QuarantineReason::OwnershipMismatch => DiscrepancyKind::OwnershipMismatch,
        QuarantineReason::ProviderUnavailable => DiscrepancyKind::ProviderUnavailable,
        QuarantineReason::SourceSizeMismatch | QuarantineReason::SourceChecksumMismatch => {
            DiscrepancyKind::CorruptSource
        }
        QuarantineReason::DestinationConflict => match side {
            ObjectSide::Source => DiscrepancyKind::CorruptSource,
            ObjectSide::Target => DiscrepancyKind::CorruptTarget,
        },
    }
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum BackfillError {
    #[error("the backfill manifest is invalid")]
    InvalidManifest,
    #[error("the storage identity does not match the manifest")]
    IdentityMismatch,
    #[error("the checkpoint does not match the immutable manifest")]
    CheckpointMismatch,
    #[error("the backfill policy is invalid")]
    InvalidPolicy,
    #[error("the quarantine disposition is invalid")]
    InvalidDisposition,
    #[error("the backfill state conflicts with the requested operation")]
    StateConflict,
    #[error("storage inventory is temporarily unavailable")]
    InventoryUnavailable,
}

impl fmt::Debug for BackfillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidManifest => "InvalidManifest",
            Self::IdentityMismatch => "IdentityMismatch",
            Self::CheckpointMismatch => "CheckpointMismatch",
            Self::InvalidPolicy => "InvalidPolicy",
            Self::InvalidDisposition => "InvalidDisposition",
            Self::StateConflict => "StateConflict",
            Self::InventoryUnavailable => "InventoryUnavailable",
        })
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::ObjectVersion;

    use super::*;

    struct Fixture {
        tenant: TenantId,
        source_identity: StorageIdentity,
        target_identity: StorageIdentity,
        source: MemoryBackfillObjectStore,
        target: MemoryBackfillObjectStore,
        manifest: BackfillManifest,
    }

    fn checksum(value: &[u8]) -> ChecksumSha256 {
        sha256(value).expect("valid checksum")
    }

    fn content_type() -> ContentType {
        ContentType::parse("video/mp4").expect("valid content type")
    }

    fn storage_identity(provider: StorageProvider, fill: char) -> StorageIdentity {
        StorageIdentity::new(
            provider,
            "test-region",
            ChecksumSha256::parse(fill.to_string().repeat(64)).expect("identity digest"),
        )
        .expect("storage identity")
    }

    fn object_key(tenant: TenantId, video: VideoId, version: u32, file_name: &str) -> ObjectKey {
        ObjectKey::for_video(
            tenant,
            video,
            ObjectRole::Source,
            ObjectVersion::new(version).expect("version"),
            file_name,
        )
        .expect("object key")
    }

    fn entry(
        tenant: TenantId,
        video: VideoId,
        index: usize,
        bytes: &[u8],
        probe: bool,
    ) -> ManifestEntry {
        ManifestEntry::new(
            tenant,
            video,
            ObjectRole::Source,
            object_key(tenant, video, 1, &format!("legacy-{index}.mp4")),
            object_key(tenant, video, 2, &format!("canonical-{index}.mp4")),
            ByteSize::new(u64::try_from(bytes.len()).expect("length")).expect("size"),
            IntegrityExpectation::new(Some(checksum(bytes)), None, probe)
                .expect("integrity expectation"),
        )
        .expect("manifest entry")
    }

    fn fixture(count: usize) -> Fixture {
        let tenant = TenantId::new();
        let source_identity = storage_identity(StorageProvider::S3, 'a');
        let target_identity = storage_identity(StorageProvider::R2, 'b');
        let source = MemoryBackfillObjectStore::default();
        let target = MemoryBackfillObjectStore::default();
        let mut entries = Vec::new();
        for index in 0..count {
            let video = VideoId::new();
            let bytes = format!("valid-media-{index}").into_bytes();
            let item = entry(tenant, video, index, &bytes, true);
            source
                .seed(tenant, item.source_key.clone(), bytes, content_type(), None)
                .expect("seed source");
            entries.push(item);
        }
        let manifest = BackfillManifest::new(
            "0.1.0-test",
            source_identity.clone(),
            target_identity.clone(),
            entries,
        )
        .expect("manifest");
        Fixture {
            tenant,
            source_identity,
            target_identity,
            source,
            target,
            manifest,
        }
    }

    fn coordinator(fixture: &Fixture) -> BackfillCoordinator<'_> {
        BackfillCoordinator::new(
            &fixture.source,
            &fixture.target,
            &BasicMediaProbe,
            fixture.source_identity.clone(),
            fixture.target_identity.clone(),
        )
    }

    fn policy(concurrency: usize, attempts: u8, entries: usize) -> BackfillPolicy {
        BackfillPolicy::new(concurrency, attempts, entries, 1_000_000).expect("policy")
    }

    #[test]
    fn immutable_manifest_rejects_tenant_crossings_and_redacts_identities() {
        let tenant = TenantId::new();
        let other = TenantId::new();
        let video = VideoId::new();
        let result = ManifestEntry::new(
            tenant,
            video,
            ObjectRole::Source,
            object_key(other, video, 1, "legacy.mp4"),
            object_key(tenant, video, 2, "canonical.mp4"),
            ByteSize::new(5).expect("size"),
            IntegrityExpectation::new(Some(checksum(b"frame")), None, false).expect("integrity"),
        );
        assert_eq!(result, Err(BackfillError::InvalidManifest));

        let fixture = fixture(1);
        let debug = format!("{:?}", fixture.manifest);
        assert!(!debug.contains(&fixture.tenant.to_string()));
        assert!(!debug.contains(fixture.manifest.entries[0].source_key.as_str()));
        assert!(!debug.contains(&"a".repeat(64)));
        assert!(debug.contains("entry_count: 1"));
    }

    #[tokio::test]
    async fn completed_manifest_reruns_without_duplicate_writes() {
        let fixture = fixture(1);
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let first = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(2, 3, 10),
            )
            .await
            .expect("first run");
        assert_eq!(first.copied(), 1);
        assert_eq!(fixture.target.successful_writes(), 1);
        assert_eq!(checkpoint.status(0), Some(BackfillStatus::Copied));
        let restored = BackfillCheckpoint::restore(
            &fixture.manifest,
            fixture.manifest.digest().clone(),
            checkpoint.entries().to_vec(),
            checkpoint.revision(),
        )
        .expect("restore checkpoint");
        assert_eq!(restored, checkpoint);

        let replay = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(2, 3, 10),
            )
            .await
            .expect("replay");
        assert_eq!(replay.attempted(), 0);
        assert_eq!(fixture.target.successful_writes(), 1);
        let reconciliation = coordinator(&fixture)
            .reconcile(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &checkpoint,
            )
            .await
            .expect("reconcile");
        assert!(reconciliation.clean());
        assert_eq!(reconciliation.verified_checksums(), 2);
        assert_eq!(
            reconciliation.observed_source_role_count(ObjectRole::Source),
            1
        );
        assert_eq!(
            reconciliation.observed_target_role_count(ObjectRole::Source),
            1
        );
    }

    #[tokio::test]
    async fn interrupted_batches_resume_with_bounded_concurrency() {
        let fixture = fixture(4);
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let first = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(2, 3, 2),
            )
            .await
            .expect("partial run");
        assert_eq!(first.copied(), 2);
        assert_eq!(first.remaining(), 2);
        assert!(first.interrupted());
        assert!((2..=2).contains(&fixture.target.max_active_operations()));

        let second = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(2, 3, 2),
            )
            .await
            .expect("resumed run");
        assert_eq!(second.copied(), 2);
        assert_eq!(second.remaining(), 0);
        assert_eq!(fixture.target.successful_writes(), 4);
        assert!((1..=2).contains(&fixture.source.max_active_operations()));
    }

    #[tokio::test]
    async fn uncertain_post_commit_failure_reuses_atomic_destination() {
        let fixture = fixture(1);
        fixture
            .target
            .fail_next(StorageOperation::PutAfterCommit, 1)
            .expect("inject failure");
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let report = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 3, 1),
            )
            .await
            .expect("run");
        assert_eq!(report.reused(), 1);
        assert_eq!(report.retries(), 1);
        assert_eq!(checkpoint.attempts(0), Some(2));
        assert_eq!(fixture.target.successful_writes(), 1);
    }

    #[tokio::test]
    async fn missing_source_is_quarantined_then_owner_approved_retry_resumes() {
        let mut fixture = fixture(1);
        let source_key = fixture.manifest.entries[0].source_key.clone();
        fixture.source = MemoryBackfillObjectStore::default();
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let first = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 2, 1),
            )
            .await
            .expect("run");
        assert_eq!(first.quarantined(), 1);
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::MissingSource,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );

        checkpoint
            .approve_quarantine(0, QuarantineDisposition::RetryApproved)
            .expect("approve retry");
        fixture
            .source
            .seed(
                fixture.tenant,
                source_key,
                b"valid-media-0".to_vec(),
                content_type(),
                None,
            )
            .expect("restore source");
        let resumed = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 2, 1),
            )
            .await
            .expect("resume");
        assert_eq!(resumed.copied(), 1);
        assert_eq!(checkpoint.attempts(0), Some(2));
    }

    #[tokio::test]
    async fn corrupt_and_unplayable_sources_are_quarantined_without_publication() {
        let mut corrupt = fixture(1);
        let key = corrupt.manifest.entries[0].source_key.clone();
        corrupt.source = MemoryBackfillObjectStore::default();
        corrupt
            .source
            .seed(
                corrupt.tenant,
                key,
                b"invalid-media".to_vec(),
                content_type(),
                None,
            )
            .expect("seed corrupt source");
        let mut checkpoint = BackfillCheckpoint::new(&corrupt.manifest);
        coordinator(&corrupt)
            .run(
                BackfillScope::tenant(corrupt.tenant),
                &corrupt.manifest,
                &mut checkpoint,
                policy(1, 1, 1),
            )
            .await
            .expect("run");
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::SourceChecksumMismatch,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );
        assert_eq!(corrupt.target.successful_writes(), 0);

        let tenant = TenantId::new();
        let video = VideoId::new();
        let bytes = b"CORRUPT-media";
        let source = MemoryBackfillObjectStore::default();
        let target = MemoryBackfillObjectStore::default();
        let item = entry(tenant, video, 0, bytes, true);
        source
            .seed(
                tenant,
                item.source_key.clone(),
                bytes.to_vec(),
                content_type(),
                None,
            )
            .expect("seed unplayable source");
        let source_identity = storage_identity(StorageProvider::S3, 'c');
        let target_identity = storage_identity(StorageProvider::R2, 'd');
        let manifest = BackfillManifest::new(
            "probe-test",
            source_identity.clone(),
            target_identity.clone(),
            vec![item],
        )
        .expect("manifest");
        let service = BackfillCoordinator::new(
            &source,
            &target,
            &BasicMediaProbe,
            source_identity,
            target_identity,
        );
        let mut checkpoint = BackfillCheckpoint::new(&manifest);
        service
            .run(
                BackfillScope::tenant(tenant),
                &manifest,
                &mut checkpoint,
                policy(1, 1, 1),
            )
            .await
            .expect("run");
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::SourceProbeFailed,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );
    }

    #[tokio::test]
    async fn conflicting_destination_is_never_overwritten() {
        let fixture = fixture(1);
        let target_key = fixture.manifest.entries[0].target_key.clone();
        fixture
            .target
            .seed(
                fixture.tenant,
                target_key.clone(),
                b"other-content".to_vec(),
                content_type(),
                None,
            )
            .expect("seed conflict");
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 3, 1),
            )
            .await
            .expect("run");
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::DestinationConflict,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );
        assert_eq!(fixture.target.successful_writes(), 0);
        assert_eq!(
            fixture
                .target
                .read(&target_key)
                .await
                .expect("read")
                .expect("target")
                .bytes(),
            b"other-content"
        );
    }

    #[tokio::test]
    async fn object_ownership_mismatch_is_quarantined_and_debug_is_private() {
        let mut fixture = fixture(1);
        let other = TenantId::new();
        let item = &fixture.manifest.entries[0];
        fixture.source = MemoryBackfillObjectStore::default();
        fixture
            .source
            .seed(
                other,
                item.source_key.clone(),
                b"valid-media-0".to_vec(),
                content_type(),
                None,
            )
            .expect("seed mismatched owner");
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 1, 1),
            )
            .await
            .expect("run");
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::OwnershipMismatch,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );
        let debug = format!("{checkpoint:?}");
        assert!(!debug.contains(&fixture.tenant.to_string()));
        assert!(!debug.contains(&other.to_string()));
    }

    #[tokio::test]
    async fn reconciliation_finds_duplicates_missing_and_orphan_targets_without_mutation() {
        let tenant = TenantId::new();
        let first_video = VideoId::new();
        let second_video = VideoId::new();
        let bytes = b"valid-shared-source";
        let first = entry(tenant, first_video, 0, bytes, false);
        let mut second = entry(tenant, second_video, 1, bytes, false);
        second.source_key = first.source_key.clone();
        let source = MemoryBackfillObjectStore::default();
        let target = MemoryBackfillObjectStore::default();
        source
            .seed(
                tenant,
                first.source_key.clone(),
                bytes.to_vec(),
                content_type(),
                None,
            )
            .expect("seed source");
        let orphan = object_key(tenant, VideoId::new(), 1, "orphan.mp4");
        target
            .seed(tenant, orphan, b"orphan".to_vec(), content_type(), None)
            .expect("seed orphan");
        let source_identity = storage_identity(StorageProvider::S3, 'e');
        let target_identity = storage_identity(StorageProvider::R2, 'f');
        let manifest = BackfillManifest::new(
            "reconcile-test",
            source_identity.clone(),
            target_identity.clone(),
            vec![first, second],
        )
        .expect("manifest");
        let checkpoint = BackfillCheckpoint::new(&manifest);
        let service = BackfillCoordinator::new(
            &source,
            &target,
            &BasicMediaProbe,
            source_identity,
            target_identity,
        );
        let report = service
            .reconcile(BackfillScope::tenant(tenant), &manifest, &checkpoint)
            .await
            .expect("reconcile");
        let kinds = report.discrepancy_kinds().collect::<Vec<_>>();
        assert_eq!(report.expected_objects(), 2);
        assert_eq!(report.observed_source_objects(), 2);
        assert_eq!(report.observed_target_objects(), 0);
        assert_eq!(report.role_count(ObjectRole::Source), 2);
        assert!(kinds.contains(&DiscrepancyKind::DuplicateSource));
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == DiscrepancyKind::MissingTarget)
                .count(),
            2
        );
        assert!(kinds.contains(&DiscrepancyKind::OrphanTarget));
        let plan = report.repair_plan();
        assert!(plan.is_dry_run());
        assert!(
            plan.actions()
                .iter()
                .any(|action| matches!(action, RepairAction::ReviewOrphanTarget))
        );
        assert_eq!(target.successful_writes(), 0);
    }

    #[tokio::test]
    async fn reconciliation_detects_corrupt_published_target_and_checkpoint_drift() {
        let fixture = fixture(1);
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 2, 1),
            )
            .await
            .expect("copy");
        fixture
            .target
            .seed(
                fixture.tenant,
                fixture.manifest.entries[0].target_key.clone(),
                b"corrupt-target".to_vec(),
                content_type(),
                None,
            )
            .expect("corrupt target");
        let report = coordinator(&fixture)
            .reconcile(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &checkpoint,
            )
            .await
            .expect("reconcile");
        let kinds = report.discrepancy_kinds().collect::<Vec<_>>();
        assert!(kinds.contains(&DiscrepancyKind::CorruptTarget));
        assert!(kinds.contains(&DiscrepancyKind::CheckpointMismatch));
        assert!(!report.clean());
    }

    #[tokio::test]
    async fn retry_exhaustion_redacts_provider_details() {
        let fixture = fixture(1);
        fixture
            .source
            .fail_next(StorageOperation::Read, 5)
            .expect("inject outage");
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let report = coordinator(&fixture)
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 2, 1),
            )
            .await
            .expect("run");
        assert_eq!(report.retries(), 1);
        assert_eq!(
            checkpoint.status(0),
            Some(BackfillStatus::Quarantined {
                reason: QuarantineReason::ProviderUnavailable,
                disposition: QuarantineDisposition::PendingOwnerDecision,
            })
        );
        let debug = format!("{checkpoint:?} {report:?}");
        assert!(!debug.contains("private.example"));
        assert!(!debug.contains("secret"));
    }

    #[tokio::test]
    async fn manifest_and_checkpoint_are_bound_to_exact_storage_authorities() {
        let fixture = fixture(1);
        let wrong_identity = storage_identity(StorageProvider::R2, '9');
        let service = BackfillCoordinator::new(
            &fixture.source,
            &fixture.target,
            &BasicMediaProbe,
            fixture.source_identity.clone(),
            wrong_identity,
        );
        let mut checkpoint = BackfillCheckpoint::new(&fixture.manifest);
        let error = service
            .run(
                BackfillScope::tenant(fixture.tenant),
                &fixture.manifest,
                &mut checkpoint,
                policy(1, 1, 1),
            )
            .await
            .expect_err("identity mismatch");
        assert_eq!(error, BackfillError::IdentityMismatch);
        assert_eq!(format!("{error:?}"), "IdentityMismatch");
        assert_eq!(
            format!("{error}"),
            "the storage identity does not match the manifest"
        );
    }
}
