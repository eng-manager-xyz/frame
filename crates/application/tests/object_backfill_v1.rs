use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use frame_application::{
    BackfillProcessOutcomeV1, ObjectBackfillCoordinatorV1, ObjectBackfillErrorV1,
};
use frame_domain::{
    BackfillContractErrorV1, BackfillCredentialRefV1, BackfillDestinationVersionV1,
    BackfillDiscrepancyKindV1, BackfillEntryIdV1, BackfillEntryStatusV1, BackfillExecutionPolicyV1,
    BackfillFailureClassV1, BackfillInventorySideV1, BackfillManifestIdV1,
    BackfillMediaProbeModeV1, BackfillMediaProbePolicyV1, BackfillOperationIdV1,
    BackfillOwnerApprovalIdV1, BackfillOwnerApprovalRecordV1, BackfillOwnerDispositionV1,
    BackfillProviderCapabilitiesV1, BackfillProviderChecksumV1, BackfillProviderLocatorV1,
    BackfillProviderV1, BackfillRunStateV1, BackfillSourceReferenceV1,
    BackfillSourceRetentionStateV1, BackfillStorageAuthorityV1, BackfillWorkerIdV1, ByteSize,
    ChecksumSha256, ContentType, DurationMillis, MediaProfileVersion, NormalizedTransformProfile,
    ObjectBackfillManifestEntryV1, ObjectBackfillManifestV1, ObjectRevision, ObjectRole,
    ScopedObjectKey, StorageFileExtension, TenantId, TimestampMillis, TransformProfile,
    TransformProfileName, VideoId, VideoObjectDescriptor, backfill_disposition_approval_scope_v1,
    backfill_source_release_approval_scope_v1,
};
use frame_ports::{
    BackfillCancellationPortV1, BackfillChunkV1, BackfillClockPortV1, BackfillCommitFenceV1,
    BackfillCommitReceiptV1, BackfillConditionalCreateV1, BackfillCreateSpecV1,
    BackfillDestinationPortV1, BackfillInventoryCursorV1, BackfillInventoryPageV1,
    BackfillJournalPortV1, BackfillMediaProbePortV1, BackfillObjectMetadataV1,
    BackfillOpenedReadV1, BackfillOwnerApprovalCapabilityV1, BackfillOwnerApprovalPortV1,
    BackfillPortErrorV1, BackfillProbeReceiptV1, BackfillProbeSessionV1, BackfillProviderAccessV1,
    BackfillReadBodyV1, BackfillRuntimeBindingsV1, BackfillSourcePortV1,
    BackfillThrottleDecisionV1, BackfillThrottlePortV1, BackfillWriteBodyV1,
    MemoryBackfillJournalPortV1, MemoryBackfillManifestPortV1,
};

fn timestamp(value: i64) -> TimestampMillis {
    TimestampMillis::new(value).expect("timestamp")
}

fn policy(max_attempts: u16) -> BackfillExecutionPolicyV1 {
    bounded_policy(max_attempts, 4, 1_000, 50_000_000, 50_000)
}

fn bounded_policy(
    max_attempts: u16,
    max_concurrency: u16,
    max_entries: u64,
    max_bytes: u64,
    max_cost: u64,
) -> BackfillExecutionPolicyV1 {
    BackfillExecutionPolicyV1::new(
        max_concurrency,
        max_attempts,
        max_entries,
        max_bytes,
        max_cost,
        5_000_000,
        1_000,
        DurationMillis::new(10).expect("retry base"),
        DurationMillis::new(100).expect("retry max"),
        2,
        DurationMillis::new(1_000).expect("circuit cooldown"),
        DurationMillis::new(100).expect("lease ttl"),
        ByteSize::new(8).expect("chunk bound"),
    )
    .expect("policy")
}

fn half_open_policy() -> BackfillExecutionPolicyV1 {
    BackfillExecutionPolicyV1::new(
        4,
        3,
        1_000,
        50_000_000,
        50_000,
        5_000_000,
        1_000,
        DurationMillis::new(10).expect("retry base"),
        DurationMillis::new(100).expect("retry max"),
        1,
        DurationMillis::new(1_000).expect("circuit cooldown"),
        DurationMillis::new(100).expect("lease ttl"),
        ByteSize::new(8).expect("chunk bound"),
    )
    .expect("half-open policy")
}

fn authority(provider: BackfillProviderV1, marker: u8) -> BackfillStorageAuthorityV1 {
    BackfillStorageAuthorityV1::new(
        provider,
        "us-rehearsal-1",
        BackfillProviderLocatorV1::parse(format!("fixture-{marker:02x}"))
            .expect("provider locator"),
        ChecksumSha256::parse(format!("{marker:02x}").repeat(32)).expect("fingerprint"),
    )
    .expect("authority")
}

fn descriptor_for_role(role: ObjectRole) -> VideoObjectDescriptor {
    let extension = |value| StorageFileExtension::parse(value).expect("extension");
    match role {
        ObjectRole::Source => VideoObjectDescriptor::Source {
            extension: extension("mp4"),
        },
        ObjectRole::RecordingSegment => VideoObjectDescriptor::RecordingSegment {
            index: frame_domain::RecordingSegmentIndex::new(0).expect("segment"),
            extension: extension("webm"),
        },
        ObjectRole::Thumbnail => VideoObjectDescriptor::Thumbnail {
            extension: extension("png"),
        },
        ObjectRole::Screenshot => VideoObjectDescriptor::Screenshot {
            extension: extension("png"),
        },
        ObjectRole::Preview => VideoObjectDescriptor::Preview {
            extension: extension("mp4"),
        },
        ObjectRole::Spritesheet => VideoObjectDescriptor::Spritesheet {
            extension: extension("jpg"),
        },
        ObjectRole::Audio => VideoObjectDescriptor::Audio {
            extension: extension("m4a"),
        },
        ObjectRole::Caption => VideoObjectDescriptor::Caption {
            extension: extension("vtt"),
        },
        ObjectRole::Export => VideoObjectDescriptor::Export {
            extension: extension("mp4"),
        },
        ObjectRole::Manifest => VideoObjectDescriptor::Manifest,
    }
}

fn target_key(tenant: TenantId, video: VideoId, role: ObjectRole, index: usize) -> ScopedObjectKey {
    let revision = ObjectRevision::new(2).expect("revision");
    match role {
        ObjectRole::Source | ObjectRole::RecordingSegment => {
            ScopedObjectKey::source(tenant, video, revision, descriptor_for_role(role))
                .expect("source key")
        }
        ObjectRole::Manifest => {
            let profile = TransformProfile::new(
                TransformProfileName::parse(format!("manifest-{index}")).expect("profile name"),
                MediaProfileVersion::new(1).expect("profile version"),
                NormalizedTransformProfile::parse("codec=h264;container=mp4")
                    .expect("normalized profile"),
                VideoObjectDescriptor::Preview {
                    extension: StorageFileExtension::parse("mp4").expect("extension"),
                },
            )
            .expect("profile");
            ScopedObjectKey::manifest(tenant, video, revision, &profile).expect("manifest key")
        }
        _ => {
            let profile = TransformProfile::new(
                TransformProfileName::parse(format!("role-{index}")).expect("profile name"),
                MediaProfileVersion::new(1).expect("profile version"),
                NormalizedTransformProfile::parse("codec=h264;container=mp4")
                    .expect("normalized profile"),
                descriptor_for_role(role),
            )
            .expect("profile");
            ScopedObjectKey::derivative(tenant, video, revision, &profile).expect("derivative key")
        }
    }
}

fn manifest_entry(
    tenant: TenantId,
    role: ObjectRole,
    index: usize,
    bytes: &[u8],
) -> ObjectBackfillManifestEntryV1 {
    let video = VideoId::new();
    ObjectBackfillManifestEntryV1::new(
        BackfillEntryIdV1::new(),
        tenant,
        video,
        role,
        BackfillSourceReferenceV1::parse(format!("legacy/{index}/{}.bin", role.path_segment()))
            .expect("source reference"),
        target_key(tenant, video, role, index),
        ByteSize::new(u64::try_from(bytes.len()).expect("length")).expect("size"),
        ChecksumSha256::digest_bytes(bytes),
        Some(
            BackfillProviderChecksumV1::parse(format!("opaque-etag-{index}-2"))
                .expect("opaque provider checksum"),
        ),
        ContentType::parse(match role {
            ObjectRole::Thumbnail | ObjectRole::Spritesheet => "image/png",
            ObjectRole::Audio => "audio/mp4",
            ObjectRole::Manifest => "application/json",
            _ => "video/mp4",
        })
        .expect("content type"),
        BackfillMediaProbePolicyV1::new(1, BackfillMediaProbeModeV1::Required)
            .expect("probe policy"),
    )
    .expect("manifest entry")
}

fn full_capabilities() -> BackfillProviderCapabilitiesV1 {
    BackfillProviderCapabilitiesV1 {
        streaming_read: true,
        streaming_conditional_create: true,
        exact_head_after_write: true,
        immutable_versions: true,
        independent_inventory: true,
        snapshot_inventory: true,
        cancelable_staging_write: true,
        live_commit_fencing: true,
    }
}

#[derive(Debug, Clone)]
enum ReadFault {
    Truncate(usize),
    Extra(Vec<u8>),
    Corrupt(usize),
    OversizedChunk,
    MidstreamOutage(usize),
    InvalidEmptyChunk,
    Open(BackfillPortErrorV1),
}

#[derive(Debug, Clone)]
enum InventoryFault {
    SnapshotMutation,
    PageIndex(u64),
    RepeatCursor,
    EmptyPageWithNext,
    CrossTenant(Box<BackfillObjectMetadataV1>),
}

#[derive(Clone)]
struct FakeObject {
    metadata: BackfillObjectMetadataV1,
    bytes: Vec<u8>,
}

struct FakeProviderInner {
    authority: BackfillStorageAuthorityV1,
    capabilities: Mutex<BackfillProviderCapabilitiesV1>,
    objects: Mutex<HashMap<String, FakeObject>>,
    read_faults: Mutex<HashMap<String, VecDeque<ReadFault>>>,
    inventory_extras: Mutex<Vec<BackfillObjectMetadataV1>>,
    inventory_fault: Mutex<Option<InventoryFault>>,
    inventory_calls: AtomicUsize,
    commit_lost_ack: AtomicUsize,
    create_count: AtomicUsize,
    write_cancel_count: AtomicUsize,
    read_release_count: AtomicUsize,
    next_version: AtomicU64,
    commit_blocked: AtomicBool,
    commit_waiting: AtomicBool,
}

#[derive(Clone)]
struct FakeProvider(Arc<FakeProviderInner>);

impl FakeProvider {
    fn new(authority: BackfillStorageAuthorityV1) -> Self {
        Self(Arc::new(FakeProviderInner {
            authority,
            capabilities: Mutex::new(full_capabilities()),
            objects: Mutex::new(HashMap::new()),
            read_faults: Mutex::new(HashMap::new()),
            inventory_extras: Mutex::new(Vec::new()),
            inventory_fault: Mutex::new(None),
            inventory_calls: AtomicUsize::new(0),
            commit_lost_ack: AtomicUsize::new(0),
            create_count: AtomicUsize::new(0),
            write_cancel_count: AtomicUsize::new(0),
            read_release_count: AtomicUsize::new(0),
            next_version: AtomicU64::new(1),
            commit_blocked: AtomicBool::new(false),
            commit_waiting: AtomicBool::new(false),
        }))
    }

    fn source_storage_key(reference: &BackfillSourceReferenceV1) -> String {
        format!("source:{}", reference.as_str())
    }

    fn target_storage_key(key: &ScopedObjectKey) -> String {
        format!("target:{}", key.as_str())
    }

    fn seed_source(&self, entry: &ObjectBackfillManifestEntryV1, bytes: Vec<u8>) {
        let metadata = BackfillObjectMetadataV1::source(
            self.0.authority.authority_fingerprint().clone(),
            entry.tenant_id(),
            entry.video_id(),
            entry.role(),
            entry.source_reference().clone(),
            entry.expected_size(),
            entry.content_type().clone(),
            Some(entry.strong_sha256().clone()),
            entry.source_provider_checksum().cloned(),
        )
        .expect("source metadata");
        self.0.objects.lock().expect("objects lock").insert(
            Self::source_storage_key(entry.source_reference()),
            FakeObject { metadata, bytes },
        );
    }

    fn seed_target(&self, entry: &ObjectBackfillManifestEntryV1, bytes: Vec<u8>) {
        self.seed_target_for_operation(entry, bytes, BackfillOperationIdV1::new());
    }

    fn seed_target_for_operation(
        &self,
        entry: &ObjectBackfillManifestEntryV1,
        bytes: Vec<u8>,
        operation_id: BackfillOperationIdV1,
    ) {
        let version = self.0.next_version.fetch_add(1, Ordering::SeqCst);
        let metadata = BackfillObjectMetadataV1::target(
            self.0.authority.authority_fingerprint().clone(),
            entry.tenant_id(),
            entry.video_id(),
            entry.role(),
            entry.target_key().clone(),
            entry.expected_size(),
            entry.content_type().clone(),
            Some(entry.strong_sha256().clone()),
            None,
            BackfillDestinationVersionV1::parse(format!("version-{version}")).expect("version"),
            Some(operation_id),
        )
        .expect("target metadata");
        self.0.objects.lock().expect("objects lock").insert(
            Self::target_storage_key(entry.target_key()),
            FakeObject { metadata, bytes },
        );
    }

    fn queue_read_fault(&self, key: String, fault: ReadFault) {
        self.0
            .read_faults
            .lock()
            .expect("fault lock")
            .entry(key)
            .or_default()
            .push_back(fault);
    }

    fn queue_source_fault(&self, entry: &ObjectBackfillManifestEntryV1, fault: ReadFault) {
        self.queue_read_fault(Self::source_storage_key(entry.source_reference()), fault);
    }

    fn queue_target_fault(&self, entry: &ObjectBackfillManifestEntryV1, fault: ReadFault) {
        self.queue_read_fault(Self::target_storage_key(entry.target_key()), fault);
    }

    fn set_inventory_fault(&self, fault: InventoryFault) {
        *self.0.inventory_fault.lock().expect("inventory fault lock") = Some(fault);
        self.0.inventory_calls.store(0, Ordering::SeqCst);
    }

    fn set_commit_lost_ack(&self, count: usize) {
        self.0.commit_lost_ack.store(count, Ordering::SeqCst);
    }

    fn create_count(&self) -> usize {
        self.0.create_count.load(Ordering::SeqCst)
    }

    fn write_cancel_count(&self) -> usize {
        self.0.write_cancel_count.load(Ordering::SeqCst)
    }

    fn read_release_count(&self) -> usize {
        self.0.read_release_count.load(Ordering::SeqCst)
    }

    fn block_commit(&self) {
        self.0.commit_blocked.store(true, Ordering::SeqCst);
    }

    fn release_commit(&self) {
        self.0.commit_blocked.store(false, Ordering::SeqCst);
    }

    fn commit_waiting(&self) -> bool {
        self.0.commit_waiting.load(Ordering::SeqCst)
    }

    fn remove_source(&self, entry: &ObjectBackfillManifestEntryV1) {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .remove(&Self::source_storage_key(entry.source_reference()));
    }

    fn remove_target(&self, entry: &ObjectBackfillManifestEntryV1) {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .remove(&Self::target_storage_key(entry.target_key()));
    }

    fn replace_target_bytes(&self, entry: &ObjectBackfillManifestEntryV1, bytes: Vec<u8>) {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .get_mut(&Self::target_storage_key(entry.target_key()))
            .expect("target object")
            .bytes = bytes;
    }

    fn replace_source_bytes(&self, entry: &ObjectBackfillManifestEntryV1, bytes: Vec<u8>) {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .get_mut(&Self::source_storage_key(entry.source_reference()))
            .expect("source object")
            .bytes = bytes;
    }

    fn replace_target_operation(
        &self,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
    ) {
        let key = Self::target_storage_key(entry.target_key());
        let mut objects = self.0.objects.lock().expect("objects lock");
        let object = objects.get_mut(&key).expect("target object");
        let version = object
            .metadata
            .destination_version()
            .cloned()
            .expect("target version");
        object.metadata = BackfillObjectMetadataV1::target(
            self.0.authority.authority_fingerprint().clone(),
            entry.tenant_id(),
            entry.video_id(),
            entry.role(),
            entry.target_key().clone(),
            entry.expected_size(),
            entry.content_type().clone(),
            Some(entry.strong_sha256().clone()),
            None,
            version,
            Some(operation_id),
        )
        .expect("replacement target metadata");
    }

    fn replace_source_owner(&self, entry: &ObjectBackfillManifestEntryV1, owner: TenantId) {
        let key = Self::source_storage_key(entry.source_reference());
        let mut objects = self.0.objects.lock().expect("objects lock");
        let object = objects.get_mut(&key).expect("source object");
        object.metadata = BackfillObjectMetadataV1::source(
            self.0.authority.authority_fingerprint().clone(),
            owner,
            entry.video_id(),
            entry.role(),
            entry.source_reference().clone(),
            entry.expected_size(),
            entry.content_type().clone(),
            Some(entry.strong_sha256().clone()),
            entry.source_provider_checksum().cloned(),
        )
        .expect("mismatched metadata");
    }

    fn add_inventory_extra(&self, metadata: BackfillObjectMetadataV1) {
        self.0
            .inventory_extras
            .lock()
            .expect("inventory lock")
            .push(metadata);
    }

    fn metadata_for_source(
        &self,
        entry: &ObjectBackfillManifestEntryV1,
    ) -> BackfillObjectMetadataV1 {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .get(&Self::source_storage_key(entry.source_reference()))
            .expect("source")
            .metadata
            .clone()
    }

    fn metadata_for_target(
        &self,
        entry: &ObjectBackfillManifestEntryV1,
    ) -> BackfillObjectMetadataV1 {
        self.0
            .objects
            .lock()
            .expect("objects lock")
            .get(&Self::target_storage_key(entry.target_key()))
            .expect("target")
            .metadata
            .clone()
    }

    fn set_capabilities(&self, capabilities: BackfillProviderCapabilitiesV1) {
        *self.0.capabilities.lock().expect("capabilities lock") = capabilities;
    }

    fn open(&self, key: &str) -> Result<BackfillOpenedReadV1, BackfillPortErrorV1> {
        let fault = self
            .0
            .read_faults
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .get_mut(key)
            .and_then(VecDeque::pop_front);
        if let Some(ReadFault::Open(error)) = fault {
            return Err(error);
        }
        let object = self
            .0
            .objects
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .get(key)
            .cloned()
            .ok_or(BackfillPortErrorV1::NotFound)?;
        let mut bytes = object.bytes;
        let mut chunk_size = 4;
        let mut fail_after = None;
        let mut invalid_empty = false;
        match fault {
            Some(ReadFault::Truncate(length)) => bytes.truncate(length),
            Some(ReadFault::Extra(extra)) => bytes.extend(extra),
            Some(ReadFault::Corrupt(index)) if index < bytes.len() => bytes[index] ^= 0xff,
            Some(ReadFault::OversizedChunk) => chunk_size = 16,
            Some(ReadFault::MidstreamOutage(chunks)) => fail_after = Some(chunks),
            Some(ReadFault::InvalidEmptyChunk) => invalid_empty = true,
            Some(ReadFault::Corrupt(_) | ReadFault::Open(_)) | None => {}
        }
        Ok(BackfillOpenedReadV1::new(
            object.metadata,
            Box::new(FakeReadBody {
                provider: self.clone(),
                bytes,
                position: 0,
                chunks: 0,
                chunk_size,
                fail_after,
                invalid_empty,
                canceled: false,
            }),
        ))
    }

    fn inventory(
        &self,
        tenant_id: TenantId,
        cursor: Option<&BackfillInventoryCursorV1>,
        limit: u16,
    ) -> Result<BackfillInventoryPageV1, BackfillPortErrorV1> {
        let mut rows = self
            .0
            .objects
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .iter()
            .filter(|(_, object)| object.metadata.owner_tenant() == tenant_id)
            .map(|(key, object)| (key.clone(), object.metadata.clone()))
            .collect::<Vec<_>>();
        for (index, metadata) in self
            .0
            .inventory_extras
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .iter()
            .enumerate()
            .filter(|(_, metadata)| metadata.owner_tenant() == tenant_id)
        {
            rows.push((format!("extra-{index}"), metadata.clone()));
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        let mut snapshot_material = Vec::new();
        for (key, metadata) in &rows {
            snapshot_material.extend_from_slice(key.as_bytes());
            snapshot_material.extend_from_slice(&metadata.logical_bytes().get().to_be_bytes());
            if let Some(checksum) = metadata.strong_sha256() {
                snapshot_material.extend_from_slice(checksum.as_str().as_bytes());
            }
            if let Some(version) = metadata.destination_version() {
                snapshot_material.extend_from_slice(version.as_str().as_bytes());
            }
        }
        let call = self.0.inventory_calls.fetch_add(1, Ordering::SeqCst);
        let fault = self
            .0
            .inventory_fault
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .clone();
        if matches!(fault, Some(InventoryFault::SnapshotMutation)) {
            snapshot_material.extend_from_slice(&call.to_be_bytes());
        }
        let snapshot = ChecksumSha256::digest_bytes(&snapshot_material);
        let start = cursor
            .map(BackfillInventoryCursorV1::expose_to_adapter)
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| BackfillPortErrorV1::InvalidResponse)?;
        if start > rows.len() {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        let end = start.saturating_add(usize::from(limit)).min(rows.len());
        let mut objects = rows[start..end]
            .iter()
            .map(|(_, metadata)| metadata.clone())
            .collect::<Vec<_>>();
        let mut next = (end < rows.len())
            .then(|| BackfillInventoryCursorV1::parse(format!("{end:020}")))
            .transpose()?;
        let mut page_index = u64::try_from(start / usize::from(limit))
            .map_err(|_| BackfillPortErrorV1::InvalidResponse)?;
        match fault {
            Some(InventoryFault::PageIndex(value)) => page_index = value,
            Some(InventoryFault::RepeatCursor) => {
                next = Some(BackfillInventoryCursorV1::parse(format!("{start:020}"))?);
                if cursor.is_some() {
                    page_index = 1;
                }
            }
            Some(InventoryFault::EmptyPageWithNext) => {
                objects.clear();
                next = Some(BackfillInventoryCursorV1::parse(format!("{start:020}"))?);
            }
            Some(InventoryFault::CrossTenant(metadata)) => objects.push(*metadata),
            Some(InventoryFault::SnapshotMutation) | None => {}
        }
        BackfillInventoryPageV1::new(objects, next, snapshot, page_index, limit)
    }
}

struct FakeReadBody {
    provider: FakeProvider,
    bytes: Vec<u8>,
    position: usize,
    chunks: usize,
    chunk_size: usize,
    fail_after: Option<usize>,
    invalid_empty: bool,
    canceled: bool,
}

#[async_trait]
impl BackfillReadBodyV1 for FakeReadBody {
    async fn next_chunk(&mut self) -> Result<Option<BackfillChunkV1>, BackfillPortErrorV1> {
        if self.invalid_empty {
            self.invalid_empty = false;
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        if self.fail_after == Some(self.chunks) {
            self.fail_after = None;
            return Err(BackfillPortErrorV1::ProviderOutage);
        }
        if self.position == self.bytes.len() {
            return Ok(None);
        }
        let end = self
            .position
            .saturating_add(self.chunk_size)
            .min(self.bytes.len());
        let chunk = BackfillChunkV1::new(self.bytes[self.position..end].to_vec())?;
        self.position = end;
        self.chunks = self.chunks.saturating_add(1);
        Ok(Some(chunk))
    }

    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1> {
        if !self.canceled {
            self.canceled = true;
            self.provider
                .0
                .read_release_count
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl Drop for FakeReadBody {
    fn drop(&mut self) {
        if !self.canceled {
            self.provider
                .0
                .read_release_count
                .fetch_add(1, Ordering::SeqCst);
            self.canceled = true;
        }
    }
}

struct FakeWriteBody {
    provider: FakeProvider,
    spec: BackfillCreateSpecV1,
    bytes: Vec<u8>,
    committed: bool,
    canceled: bool,
}

#[async_trait]
impl BackfillWriteBodyV1 for FakeWriteBody {
    async fn write_chunk(&mut self, chunk: BackfillChunkV1) -> Result<(), BackfillPortErrorV1> {
        if self.committed || self.canceled {
            return Err(BackfillPortErrorV1::Conflict);
        }
        self.bytes.extend(chunk.into_bytes());
        Ok(())
    }

    async fn commit(
        &mut self,
        _probe: Option<&BackfillProbeReceiptV1>,
        fence: &dyn BackfillCommitFenceV1,
    ) -> Result<BackfillCommitReceiptV1, BackfillPortErrorV1> {
        if self.committed || self.canceled {
            return Err(BackfillPortErrorV1::Conflict);
        }
        if u64::try_from(self.bytes.len()).ok() != Some(self.spec.expected_size().get())
            || ChecksumSha256::digest_bytes(&self.bytes) != *self.spec.expected_sha256()
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        let key = FakeProvider::target_storage_key(self.spec.target_key());
        let version_number = self.provider.0.next_version.fetch_add(1, Ordering::SeqCst);
        let version = BackfillDestinationVersionV1::parse(format!("version-{version_number}"))
            .expect("provider version");
        let metadata = BackfillObjectMetadataV1::target(
            self.spec.authority_fingerprint().clone(),
            self.spec.owner_tenant(),
            self.spec.video_id(),
            self.spec.role(),
            self.spec.target_key().clone(),
            self.spec.expected_size(),
            self.spec.content_type().clone(),
            Some(self.spec.expected_sha256().clone()),
            None,
            version.clone(),
            Some(self.spec.operation_id()),
        )?;
        if fence.entry_id() != self.spec.entry_id()
            || fence.operation_id() != self.spec.operation_id()
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        self.provider.0.commit_waiting.store(true, Ordering::SeqCst);
        while self.provider.0.commit_blocked.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        self.provider
            .0
            .commit_waiting
            .store(false, Ordering::SeqCst);
        fence.authorize_publication().await?;
        let mut objects = self
            .provider
            .0
            .objects
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?;
        if objects.contains_key(&key) {
            return Err(BackfillPortErrorV1::Conflict);
        }
        objects.insert(
            key,
            FakeObject {
                metadata,
                bytes: self.bytes.clone(),
            },
        );
        drop(objects);
        self.committed = true;
        self.provider.0.create_count.fetch_add(1, Ordering::SeqCst);
        if self
            .provider
            .0
            .commit_lost_ack
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Err(BackfillPortErrorV1::ProviderOutage);
        }
        Ok(BackfillCommitReceiptV1::new(
            version,
            self.spec.operation_id(),
        ))
    }

    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1> {
        if !self.committed && !self.canceled {
            self.canceled = true;
            self.provider
                .0
                .write_cancel_count
                .fetch_add(1, Ordering::SeqCst);
            self.bytes.clear();
        }
        Ok(())
    }
}

impl Drop for FakeWriteBody {
    fn drop(&mut self) {
        if !self.committed && !self.canceled {
            self.provider
                .0
                .write_cancel_count
                .fetch_add(1, Ordering::SeqCst);
            self.bytes.clear();
        }
    }
}

#[async_trait]
impl BackfillSourcePortV1 for FakeProvider {
    async fn capabilities(
        &self,
        access: &BackfillProviderAccessV1<'_>,
    ) -> Result<BackfillProviderCapabilitiesV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(*self
            .0
            .capabilities
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?)
    }

    async fn open_read(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        reference: &BackfillSourceReferenceV1,
        _operation_id: BackfillOperationIdV1,
    ) -> Result<BackfillOpenedReadV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        self.open(&Self::source_storage_key(reference))
    }

    async fn estimate_egress_cost_units(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        logical_bytes: ByteSize,
    ) -> Result<u64, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(logical_bytes.get().div_ceil(2_048).max(1))
    }

    async fn inventory_page(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenant_id: TenantId,
        cursor: Option<&BackfillInventoryCursorV1>,
        limit: u16,
    ) -> Result<BackfillInventoryPageV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        self.inventory(tenant_id, cursor, limit)
    }
}

#[async_trait]
impl BackfillDestinationPortV1 for FakeProvider {
    async fn capabilities(
        &self,
        access: &BackfillProviderAccessV1<'_>,
    ) -> Result<BackfillProviderCapabilitiesV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(*self
            .0
            .capabilities
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?)
    }

    async fn estimate_cost_units(
        &self,
        _access: &BackfillProviderAccessV1<'_>,
        logical_bytes: ByteSize,
    ) -> Result<u64, BackfillPortErrorV1> {
        Ok(logical_bytes.get().div_ceil(1_024).max(1))
    }

    async fn head(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        key: &ScopedObjectKey,
    ) -> Result<Option<BackfillObjectMetadataV1>, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(self
            .0
            .objects
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .get(&Self::target_storage_key(key))
            .map(|object| object.metadata.clone()))
    }

    async fn open_read(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        key: &ScopedObjectKey,
        _operation_id: BackfillOperationIdV1,
    ) -> Result<BackfillOpenedReadV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        self.open(&Self::target_storage_key(key))
    }

    async fn begin_conditional_create(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        spec: &BackfillCreateSpecV1,
    ) -> Result<BackfillConditionalCreateV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        if let Some(existing) = self
            .0
            .objects
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .get(&Self::target_storage_key(spec.target_key()))
            .cloned()
        {
            return Ok(BackfillConditionalCreateV1::AlreadyPresent(Box::new(
                existing.metadata,
            )));
        }
        Ok(BackfillConditionalCreateV1::Ready(Box::new(
            FakeWriteBody {
                provider: self.clone(),
                spec: spec.clone(),
                bytes: Vec::new(),
                committed: false,
                canceled: false,
            },
        )))
    }

    async fn inventory_page(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenant_id: TenantId,
        cursor: Option<&BackfillInventoryCursorV1>,
        limit: u16,
    ) -> Result<BackfillInventoryPageV1, BackfillPortErrorV1> {
        if access.authority() != &self.0.authority {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        self.inventory(tenant_id, cursor, limit)
    }
}

#[derive(Default)]
struct FakeProbePort {
    roles: Mutex<HashSet<ObjectRole>>,
}

#[async_trait]
impl BackfillMediaProbePortV1 for FakeProbePort {
    async fn start(
        &self,
        role: ObjectRole,
        policy: BackfillMediaProbePolicyV1,
    ) -> Result<Box<dyn BackfillProbeSessionV1>, BackfillPortErrorV1> {
        self.roles
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .insert(role);
        Ok(Box::new(FakeProbeSession {
            profile_version: policy.profile_version(),
            prefix: Vec::with_capacity(7),
            any_nonzero: false,
            canceled: false,
        }))
    }
}

struct FakeProbeSession {
    profile_version: u16,
    prefix: Vec<u8>,
    any_nonzero: bool,
    canceled: bool,
}

#[async_trait]
impl BackfillProbeSessionV1 for FakeProbeSession {
    async fn observe(&mut self, bytes: &[u8]) -> Result<(), BackfillPortErrorV1> {
        self.any_nonzero |= bytes.iter().any(|byte| *byte != 0);
        let remaining = 7_usize.saturating_sub(self.prefix.len());
        self.prefix.extend(bytes.iter().take(remaining));
        Ok(())
    }

    async fn finish(&mut self) -> Result<BackfillProbeReceiptV1, BackfillPortErrorV1> {
        if self.canceled {
            return Err(BackfillPortErrorV1::Canceled);
        }
        BackfillProbeReceiptV1::new(
            self.profile_version,
            self.any_nonzero && self.prefix.as_slice() != b"CORRUPT",
        )
    }

    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1> {
        self.canceled = true;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThrottleObservation {
    tenant_id: TenantId,
    source_provider: BackfillProviderV1,
    source_region: String,
    target_provider: BackfillProviderV1,
    target_region: String,
    bytes: Option<ByteSize>,
    now: TimestampMillis,
}

#[derive(Default)]
struct FakeThrottle {
    object_deferrals: AtomicUsize,
    byte_deferrals: AtomicUsize,
    observations: Mutex<Vec<ThrottleObservation>>,
}

impl FakeThrottle {
    fn defer_objects(&self, count: usize) {
        self.object_deferrals.store(count, Ordering::SeqCst);
    }

    fn defer_bytes(&self, count: usize) {
        self.byte_deferrals.store(count, Ordering::SeqCst);
    }

    fn observations(&self) -> Vec<ThrottleObservation> {
        self.observations
            .lock()
            .expect("throttle observations lock")
            .clone()
    }
}

#[async_trait]
impl BackfillThrottlePortV1 for FakeThrottle {
    async fn admit_object(
        &self,
        tenant_id: TenantId,
        source_provider: BackfillProviderV1,
        source_region: &str,
        target_provider: BackfillProviderV1,
        target_region: &str,
        _max_objects_per_minute: u32,
        now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1> {
        self.observations
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .push(ThrottleObservation {
                tenant_id,
                source_provider,
                source_region: source_region.to_owned(),
                target_provider,
                target_region: target_region.to_owned(),
                bytes: None,
                now,
            });
        if self
            .object_deferrals
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Ok(BackfillThrottleDecisionV1::DeferredUntil(timestamp(
                now.get() + 10,
            )));
        }
        Ok(BackfillThrottleDecisionV1::Allowed)
    }

    async fn admit_bytes(
        &self,
        tenant_id: TenantId,
        source_provider: BackfillProviderV1,
        source_region: &str,
        target_provider: BackfillProviderV1,
        target_region: &str,
        bytes: ByteSize,
        _max_bytes_per_second: u64,
        now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1> {
        self.observations
            .lock()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?
            .push(ThrottleObservation {
                tenant_id,
                source_provider,
                source_region: source_region.to_owned(),
                target_provider,
                target_region: target_region.to_owned(),
                bytes: Some(bytes),
                now,
            });
        if self
            .byte_deferrals
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Ok(BackfillThrottleDecisionV1::DeferredUntil(timestamp(
                now.get() + 10,
            )));
        }
        Ok(BackfillThrottleDecisionV1::Allowed)
    }
}

#[derive(Default)]
struct FakeCancellation(AtomicBool);

#[async_trait]
impl BackfillCancellationPortV1 for FakeCancellation {
    fn canceled(&self, _manifest_id: BackfillManifestIdV1, _entry_id: BackfillEntryIdV1) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    async fn request_manifest_abort(
        &self,
        _manifest_id: BackfillManifestIdV1,
    ) -> Result<(), BackfillPortErrorV1> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeClock {
    now: AtomicI64,
    step: AtomicI64,
}

impl FakeClock {
    fn new(now: i64) -> Self {
        Self {
            now: AtomicI64::new(now),
            step: AtomicI64::new(0),
        }
    }

    fn set(&self, now: i64) {
        self.now.store(now, Ordering::SeqCst);
    }

    fn set_step(&self, step: i64) {
        self.step.store(step, Ordering::SeqCst);
    }

    fn current(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

impl BackfillClockPortV1 for FakeClock {
    fn now(&self) -> Result<TimestampMillis, BackfillPortErrorV1> {
        let step = self.step.load(Ordering::SeqCst);
        let value = self.now.fetch_add(step, Ordering::SeqCst);
        TimestampMillis::new(value).map_err(|_| BackfillPortErrorV1::InvalidResponse)
    }
}

#[derive(Default)]
struct FakeApprovalPort {
    expired: AtomicBool,
}

impl FakeApprovalPort {
    const CAPABILITY: &'static str = "fixture-owner-capability-v1";

    fn capability() -> BackfillOwnerApprovalCapabilityV1 {
        BackfillOwnerApprovalCapabilityV1::parse(Self::CAPABILITY).expect("approval capability")
    }

    fn record(
        scope: ChecksumSha256,
        manifest: &ObjectBackfillManifestV1,
        now: TimestampMillis,
    ) -> Result<BackfillOwnerApprovalRecordV1, BackfillPortErrorV1> {
        BackfillOwnerApprovalRecordV1::new(
            BackfillOwnerApprovalIdV1::new(),
            ChecksumSha256::digest_bytes(b"authenticated-fixture-owner"),
            scope,
            manifest.created_at(),
            now.checked_add(DurationMillis::new(1_000).expect("approval lifetime"))
                .map_err(|_| BackfillPortErrorV1::InvalidResponse)?,
            now,
        )
        .map_err(|_| BackfillPortErrorV1::InvalidResponse)
    }

    fn authorize(
        &self,
        capability: &BackfillOwnerApprovalCapabilityV1,
    ) -> Result<(), BackfillPortErrorV1> {
        if self.expired.load(Ordering::SeqCst) {
            return Err(BackfillPortErrorV1::ExpiredAuthorization);
        }
        if capability.expose_to_adapter() != Self::CAPABILITY {
            return Err(BackfillPortErrorV1::NotFound);
        }
        Ok(())
    }
}

#[async_trait]
impl BackfillOwnerApprovalPortV1 for FakeApprovalPort {
    async fn verify_disposition(
        &self,
        capability: &BackfillOwnerApprovalCapabilityV1,
        manifest: &ObjectBackfillManifestV1,
        entry_id: BackfillEntryIdV1,
        tenant_id: TenantId,
        disposition: BackfillOwnerDispositionV1,
        now: TimestampMillis,
    ) -> Result<BackfillOwnerApprovalRecordV1, BackfillPortErrorV1> {
        self.authorize(capability)?;
        if !manifest
            .entry(entry_id)
            .is_some_and(|entry| entry.tenant_id() == tenant_id)
        {
            return Err(BackfillPortErrorV1::NotFound);
        }
        Self::record(
            backfill_disposition_approval_scope_v1(manifest.digest(), entry_id, disposition),
            manifest,
            now,
        )
    }

    async fn verify_source_release(
        &self,
        capability: &BackfillOwnerApprovalCapabilityV1,
        manifest: &ObjectBackfillManifestV1,
        report: &frame_domain::ObjectBackfillReconciliationReportV1,
        now: TimestampMillis,
    ) -> Result<BackfillOwnerApprovalRecordV1, BackfillPortErrorV1> {
        self.authorize(capability)?;
        Self::record(
            backfill_source_release_approval_scope_v1(manifest.digest(), report.report_digest()),
            manifest,
            now,
        )
    }
}

struct Fixture {
    manifests: MemoryBackfillManifestPortV1,
    journals: LostAckJournalPort,
    source: FakeProvider,
    target: FakeProvider,
    probes: FakeProbePort,
    throttle: FakeThrottle,
    cancellation: FakeCancellation,
    clock: FakeClock,
    approvals: FakeApprovalPort,
    source_credential: BackfillCredentialRefV1,
    target_credential: BackfillCredentialRefV1,
    manifest: ObjectBackfillManifestV1,
}

impl Fixture {
    fn new(
        roles: &[ObjectRole],
        source_provider: BackfillProviderV1,
        target_provider: BackfillProviderV1,
        marker: u8,
    ) -> Self {
        Self::new_with_policy(roles, source_provider, target_provider, marker, policy(3))
    }

    fn new_with_policy(
        roles: &[ObjectRole],
        source_provider: BackfillProviderV1,
        target_provider: BackfillProviderV1,
        marker: u8,
        execution_policy: BackfillExecutionPolicyV1,
    ) -> Self {
        let tenant = TenantId::new();
        let tenant_roles = roles
            .iter()
            .copied()
            .map(|role| (tenant, role))
            .collect::<Vec<_>>();
        Self::new_with_tenant_roles(
            &tenant_roles,
            source_provider,
            target_provider,
            marker,
            execution_policy,
        )
    }

    fn new_with_tenant_roles(
        tenant_roles: &[(TenantId, ObjectRole)],
        source_provider: BackfillProviderV1,
        target_provider: BackfillProviderV1,
        marker: u8,
        execution_policy: BackfillExecutionPolicyV1,
    ) -> Self {
        let source_authority = authority(source_provider, marker);
        let target_authority = authority(target_provider, marker.saturating_add(1));
        let source = FakeProvider::new(source_authority.clone());
        let target = FakeProvider::new(target_authority.clone());
        let entries = tenant_roles
            .iter()
            .enumerate()
            .map(|(index, (tenant, role))| {
                let bytes =
                    format!("valid-media-{}-{index}-payload", role.path_segment()).into_bytes();
                let entry = manifest_entry(*tenant, *role, index, &bytes);
                source.seed_source(&entry, bytes.clone());
                entry
            })
            .collect();
        let manifest = ObjectBackfillManifestV1::new(
            BackfillManifestIdV1::new(),
            timestamp(1),
            "object-backfill-rehearsal-1.0.0",
            format!("synthetic-code-{marker:02x}"),
            source_authority,
            target_authority,
            execution_policy,
            entries,
        )
        .expect("manifest");
        Self {
            manifests: MemoryBackfillManifestPortV1::default(),
            journals: LostAckJournalPort::default(),
            source,
            target,
            probes: FakeProbePort::default(),
            throttle: FakeThrottle::default(),
            cancellation: FakeCancellation::default(),
            clock: FakeClock::new(2),
            approvals: FakeApprovalPort::default(),
            source_credential: BackfillCredentialRefV1::parse("vault:backfill/source")
                .expect("source credential ref"),
            target_credential: BackfillCredentialRefV1::parse("vault:backfill/target")
                .expect("target credential ref"),
            manifest,
        }
    }

    fn coordinator(&self) -> ObjectBackfillCoordinatorV1<'_> {
        ObjectBackfillCoordinatorV1::new(
            &self.manifests,
            &self.journals,
            &self.source,
            &self.target,
            &self.probes,
            &self.throttle,
            &self.cancellation,
            &self.clock,
            &self.approvals,
        )
    }

    fn bindings(&self) -> BackfillRuntimeBindingsV1<'_> {
        BackfillRuntimeBindingsV1::new(
            BackfillProviderAccessV1::new(self.manifest.source(), &self.source_credential),
            BackfillProviderAccessV1::new(self.manifest.target(), &self.target_credential),
        )
    }

    fn tenant(&self) -> TenantId {
        self.manifest.entries()[0].tenant_id()
    }

    async fn initialize(&self) {
        self.coordinator()
            .initialize(&self.manifest, &self.bindings())
            .await
            .expect("initialize");
    }

    async fn process_at(&self, now: i64) -> BackfillProcessOutcomeV1 {
        self.clock.set(now);
        self.coordinator()
            .process_next(
                self.manifest.manifest_id(),
                self.tenant(),
                &self.bindings(),
                BackfillWorkerIdV1::new(),
            )
            .await
            .expect("process")
    }

    async fn journal(&self) -> frame_domain::ObjectBackfillJournalV1 {
        self.journals
            .load(self.manifest.manifest_id())
            .await
            .expect("journal load")
            .expect("journal")
    }
}

#[derive(Default)]
struct LostAckJournalPort {
    inner: MemoryBackfillJournalPortV1,
    lose_after_commit: AtomicUsize,
}

impl LostAckJournalPort {
    fn lose_after_commit(&self, count: usize) {
        self.lose_after_commit.store(count, Ordering::SeqCst);
    }
}

#[async_trait]
impl BackfillJournalPortV1 for LostAckJournalPort {
    async fn create(
        &self,
        journal: &frame_domain::ObjectBackfillJournalV1,
    ) -> Result<(), BackfillPortErrorV1> {
        self.inner.create(journal).await
    }

    async fn load(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<Option<frame_domain::ObjectBackfillJournalV1>, BackfillPortErrorV1> {
        tokio::task::yield_now().await;
        self.inner.load(manifest_id).await
    }

    async fn compare_and_swap(
        &self,
        manifest_id: BackfillManifestIdV1,
        expected_revision: u64,
        next: &frame_domain::ObjectBackfillJournalV1,
    ) -> Result<(), BackfillPortErrorV1> {
        tokio::task::yield_now().await;
        self.inner
            .compare_and_swap(manifest_id, expected_revision, next)
            .await?;
        if self
            .lose_after_commit
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            Err(BackfillPortErrorV1::ProviderOutage)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn two_independent_rehearsals_resume_idempotently_and_reconcile_exactly() {
    let rehearsals = [
        Fixture::new(
            &[ObjectRole::Source, ObjectRole::Thumbnail, ObjectRole::Audio],
            BackfillProviderV1::S3,
            BackfillProviderV1::R2,
            0x10,
        ),
        Fixture::new(
            &[ObjectRole::Preview, ObjectRole::RecordingSegment],
            BackfillProviderV1::Minio,
            BackfillProviderV1::CustomS3Compatible,
            0x20,
        ),
    ];
    let mut report_digests = HashSet::new();
    for fixture in &rehearsals {
        fixture.initialize().await;
        let first = fixture.process_at(10).await;
        assert!(matches!(first, BackfillProcessOutcomeV1::Copied(_)));
        // A new coordinator instance uses the same durable ports, simulating a restart.
        loop {
            match fixture.process_at(20).await {
                BackfillProcessOutcomeV1::Copied(_) | BackfillProcessOutcomeV1::ReusedExact(_) => {}
                BackfillProcessOutcomeV1::Completed => break,
                other => panic!("unexpected rehearsal outcome: {other:?}"),
            }
        }
        assert_eq!(
            fixture.target.create_count(),
            fixture.manifest.entries().len()
        );
        assert_eq!(
            fixture.process_at(30).await,
            BackfillProcessOutcomeV1::Completed
        );
        assert_eq!(
            fixture.target.create_count(),
            fixture.manifest.entries().len()
        );

        fixture.clock.set(40);
        let report = fixture
            .coordinator()
            .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
            .await
            .expect("reconcile");
        assert!(report.clean(), "{report:?}");
        assert_eq!(
            report.source().strong_checksums_verified(),
            u64::try_from(fixture.manifest.entries().len()).expect("count")
        );
        assert_eq!(
            report.target().media_probes_verified(),
            u64::try_from(fixture.manifest.entries().len()).expect("count")
        );
        assert!(report.dry_run_repair_plan().is_dry_run());
        println!(
            "OBJECT_BACKFILL_REHEARSAL provider={:?}->{:?} manifest_digest={} report_digest={} objects={} logical_bytes={} clean={}",
            fixture.manifest.source().provider(),
            fixture.manifest.target().provider(),
            fixture.manifest.digest().as_str(),
            report.report_digest().as_str(),
            report.expected_source().object_count(),
            report.expected_source().logical_bytes(),
            report.clean(),
        );
        report_digests.insert(report.report_digest().clone());
    }
    assert_eq!(report_digests.len(), 2);
}

#[tokio::test]
async fn lost_destination_ack_is_recovered_by_exact_streaming_post_read() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x30,
    );
    fixture.initialize().await;
    fixture.target.set_commit_lost_ack(1);
    let outcome = fixture.process_at(10).await;
    assert!(matches!(outcome, BackfillProcessOutcomeV1::ReusedExact(_)));
    assert_eq!(fixture.target.create_count(), 1);
    let journal = fixture.journal().await;
    assert_eq!(journal.state(), BackfillRunStateV1::Completed);
    assert!(matches!(
        journal.entries()[0].status(),
        BackfillEntryStatusV1::Succeeded { .. }
    ));
    assert_eq!(
        fixture.process_at(20).await,
        BackfillProcessOutcomeV1::Completed
    );
    assert_eq!(fixture.target.create_count(), 1);
}

#[tokio::test]
async fn ambiguous_claim_and_completion_cas_are_reconciled_by_exact_journal_reload() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x35,
    );
    fixture.initialize().await;
    // Both the lease claim and the terminal receipt commit, then report provider outage.
    fixture.journals.lose_after_commit(2);
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    assert_eq!(fixture.target.create_count(), 1);
    assert_eq!(
        fixture.journal().await.state(),
        BackfillRunStateV1::Completed
    );
}

#[tokio::test]
async fn crash_after_destination_commit_before_checkpoint_reuses_exact_object_on_restart() {
    let bytes = b"valid-media-source-0-payload".to_vec();
    let fixture = Fixture::new_with_policy(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x36,
        bounded_policy(1, 1, 1, u64::try_from(bytes.len()).expect("length"), 2),
    );
    fixture.initialize().await;
    let entry = &fixture.manifest.entries()[0];
    let mut claimed = fixture.journal().await;
    let revision = claimed.revision();
    let operation_id = BackfillOperationIdV1::new();
    claimed
        .claim(
            entry.entry_id(),
            BackfillWorkerIdV1::new(),
            operation_id,
            timestamp(10),
            fixture.manifest.execution_policy(),
            entry.expected_size(),
            2,
        )
        .expect("persisted claim");
    fixture
        .journals
        .compare_and_swap(fixture.manifest.manifest_id(), revision, &claimed)
        .await
        .expect("persist claim before process death");
    fixture
        .target
        .seed_target_for_operation(entry, bytes, operation_id);
    assert!(matches!(
        fixture.process_at(111).await,
        BackfillProcessOutcomeV1::ReusedExact(_)
    ));
    // The exhausted persisted admission is recovered without another attempt, write, or charge.
    assert_eq!(fixture.target.create_count(), 0);
    let journal = fixture.journal().await;
    assert_eq!(journal.state(), BackfillRunStateV1::Completed);
    assert_eq!(journal.entries()[0].attempts(), 1);
    assert_eq!(journal.usage().admitted_entries(), 1);
    assert_eq!(
        journal.usage().admitted_logical_bytes(),
        entry.expected_size().get()
    );
    assert_eq!(journal.usage().admitted_cost_units(), 2);
}

#[tokio::test]
async fn corrupt_operation_bound_target_is_never_checkpointed_as_crash_recovery() {
    let bytes = b"valid-media-source-0-payload".to_vec();
    let fixture = Fixture::new_with_policy(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x38,
        bounded_policy(1, 1, 10, 10_000, 10),
    );
    fixture.initialize().await;
    let entry = &fixture.manifest.entries()[0];
    let mut claimed = fixture.journal().await;
    let revision = claimed.revision();
    let operation_id = BackfillOperationIdV1::new();
    claimed
        .claim(
            entry.entry_id(),
            BackfillWorkerIdV1::new(),
            operation_id,
            timestamp(10),
            fixture.manifest.execution_policy(),
            entry.expected_size(),
            2,
        )
        .expect("persisted claim");
    fixture
        .journals
        .compare_and_swap(fixture.manifest.manifest_id(), revision, &claimed)
        .await
        .expect("persist claim before process death");
    fixture
        .target
        .seed_target_for_operation(entry, bytes, operation_id);
    fixture
        .target
        .replace_target_bytes(entry, b"invalid-media-target-payload".to_vec());
    assert!(matches!(
        fixture.process_at(111).await,
        BackfillProcessOutcomeV1::Quarantined {
            failure: BackfillFailureClassV1::TransferCanceled,
            ..
        }
    ));
    assert!(!matches!(
        fixture.journal().await.entries()[0].status(),
        BackfillEntryStatusV1::Succeeded { .. }
    ));
    assert_eq!(fixture.target.create_count(), 0);
}

#[tokio::test]
async fn interrupted_midstream_transfer_cleans_staging_and_resumes_once() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::Minio,
        BackfillProviderV1::R2,
        0x37,
    );
    fixture.initialize().await;
    fixture.source.queue_source_fault(
        &fixture.manifest.entries()[0],
        ReadFault::MidstreamOutage(1),
    );
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::ProviderOutage,
            ..
        }
    ));
    assert!(fixture.target.write_cancel_count() >= 1);
    assert!(matches!(
        fixture.process_at(20).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    assert_eq!(fixture.target.create_count(), 1);
    assert_eq!(
        fixture.process_at(30).await,
        BackfillProcessOutcomeV1::Completed
    );
}

#[tokio::test]
async fn truncation_corruption_extra_and_oversized_chunks_fail_closed_without_publication() {
    let cases = [
        (
            ReadFault::Truncate(3),
            BackfillFailureClassV1::TruncatedSource,
        ),
        (
            ReadFault::Corrupt(2),
            BackfillFailureClassV1::SourceChecksumMismatch,
        ),
        (
            ReadFault::Extra(vec![1, 2]),
            BackfillFailureClassV1::ExtraSourceBytes,
        ),
        (
            ReadFault::OversizedChunk,
            BackfillFailureClassV1::OversizedChunk,
        ),
        (
            ReadFault::InvalidEmptyChunk,
            BackfillFailureClassV1::SourceMetadataMismatch,
        ),
    ];
    for (index, (fault, expected)) in cases.into_iter().enumerate() {
        let fixture = Fixture::new(
            &[ObjectRole::Source],
            BackfillProviderV1::S3,
            BackfillProviderV1::R2,
            0x40 + u8::try_from(index).expect("marker"),
        );
        fixture.initialize().await;
        fixture
            .source
            .queue_source_fault(&fixture.manifest.entries()[0], fault);
        let outcome = fixture.process_at(10).await;
        assert_eq!(
            outcome,
            BackfillProcessOutcomeV1::Quarantined {
                entry_id: fixture.manifest.entries()[0].entry_id(),
                failure: expected,
            }
        );
        assert_eq!(fixture.target.create_count(), 0);
        assert!(fixture.target.write_cancel_count() >= 1);
    }
}

#[tokio::test]
async fn midstream_outage_cancellation_throttling_expiry_and_circuit_are_deterministic() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::GoogleDrive,
        BackfillProviderV1::R2,
        0x50,
    );
    fixture.initialize().await;
    fixture.throttle.defer_objects(1);
    assert_eq!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::DeferredUntil(timestamp(20))
    );
    fixture.source.queue_source_fault(
        &fixture.manifest.entries()[0],
        ReadFault::Open(BackfillPortErrorV1::ExpiredAuthorization),
    );
    assert!(matches!(
        fixture.process_at(20).await,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::ProviderExpiredAuthorization,
            ..
        }
    ));
    fixture.source.queue_source_fault(
        &fixture.manifest.entries()[0],
        ReadFault::MidstreamOutage(1),
    );
    assert!(matches!(
        fixture.process_at(30).await,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::ProviderOutage,
            ..
        }
    ));
    assert!(fixture.target.write_cancel_count() >= 1);
    assert_eq!(
        fixture.process_at(40).await,
        BackfillProcessOutcomeV1::CircuitOpenUntil(timestamp(1_030))
    );

    let canceled = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::Minio,
        BackfillProviderV1::R2,
        0x60,
    );
    canceled.initialize().await;
    canceled.cancellation.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        canceled.process_at(10).await,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::TransferCanceled,
            ..
        }
    ));
    assert!(canceled.target.write_cancel_count() >= 1);

    let bandwidth = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x61,
    );
    bandwidth.initialize().await;
    bandwidth.throttle.defer_bytes(1);
    assert!(matches!(
        bandwidth.process_at(10).await,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::ProviderThrottled,
            ..
        }
    ));
    assert!(bandwidth.target.write_cancel_count() >= 1);
}

#[tokio::test]
async fn competing_workers_share_one_claim_and_one_billable_logical_object() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x70,
    );
    fixture.initialize().await;
    let bindings_one = fixture.bindings();
    let bindings_two = fixture.bindings();
    let coordinator_one = fixture.coordinator();
    let coordinator_two = fixture.coordinator();
    fixture.clock.set(10);
    let first = coordinator_one.process_next(
        fixture.manifest.manifest_id(),
        fixture.tenant(),
        &bindings_one,
        BackfillWorkerIdV1::new(),
    );
    let second = coordinator_two.process_next(
        fixture.manifest.manifest_id(),
        fixture.tenant(),
        &bindings_two,
        BackfillWorkerIdV1::new(),
    );
    let (one, two) = tokio::join!(first, second);
    let outcomes = [one.expect("first worker"), two.expect("second worker")];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BackfillProcessOutcomeV1::Copied(_)))
            .count(),
        1
    );
    assert!(outcomes.iter().any(|outcome| matches!(
        outcome,
        BackfillProcessOutcomeV1::Idle | BackfillProcessOutcomeV1::Completed
    )));
    assert_eq!(fixture.target.create_count(), 1);
}

#[tokio::test]
async fn reconciliation_classifies_missing_duplicate_orphan_ownership_corruption_and_drift() {
    let fixture = Fixture::new(
        &[ObjectRole::Source, ObjectRole::Preview, ObjectRole::Audio],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x80,
    );
    fixture.initialize().await;
    while !matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Completed
    ) {}
    let entries = fixture.manifest.entries();
    fixture
        .target
        .replace_target_bytes(&entries[0], b"corrupt-target-data".to_vec());
    fixture.target.remove_target(&entries[1]);
    fixture
        .source
        .replace_source_owner(&entries[2], TenantId::new());
    fixture
        .source
        .add_inventory_extra(fixture.source.metadata_for_source(&entries[1]));
    fixture
        .target
        .add_inventory_extra(fixture.target.metadata_for_target(&entries[2]));

    let orphan_bytes = b"valid-orphan-payload".to_vec();
    let orphan = manifest_entry(fixture.tenant(), ObjectRole::Export, 99, &orphan_bytes);
    fixture.source.seed_source(&orphan, orphan_bytes.clone());
    fixture.target.seed_target(&orphan, orphan_bytes);
    let writes_before = fixture.target.create_count();
    fixture.clock.set(50);
    let report = fixture
        .coordinator()
        .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
        .await
        .expect("reconciliation report");
    let kinds = report
        .discrepancies()
        .iter()
        .map(frame_domain::BackfillDiscrepancyV1::kind)
        .collect::<HashSet<_>>();
    assert!(!report.clean());
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::CorruptTarget));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::MissingTarget));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::DuplicateSource));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::DuplicateTarget));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::OrphanSource));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::OrphanTarget));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::MissingSource));
    assert!(kinds.contains(&BackfillDiscrepancyKindV1::CheckpointDivergence));
    let plan = report.dry_run_repair_plan();
    assert!(plan.is_dry_run());
    assert!(!plan.actions().is_empty());
    assert_eq!(fixture.target.create_count(), writes_before);
}

#[tokio::test]
async fn pause_resume_abort_owner_disposition_and_source_retention_are_fenced() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x90,
    );
    fixture.initialize().await;
    fixture
        .coordinator()
        .pause(fixture.manifest.manifest_id())
        .await
        .expect("pause");
    assert_eq!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Paused
    );
    fixture.clock.set(20);
    fixture
        .coordinator()
        .resume(fixture.manifest.manifest_id())
        .await
        .expect("resume");
    assert!(matches!(
        fixture.process_at(21).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    fixture.clock.set(30);
    let approval = FakeApprovalPort::capability();
    let released = fixture
        .coordinator()
        .approve_source_release(
            fixture.manifest.manifest_id(),
            &fixture.bindings(),
            &approval,
        )
        .await
        .expect("source release approval");
    assert!(matches!(
        released.source_retention(),
        BackfillSourceRetentionStateV1::ReleaseApproved { .. }
    ));

    let aborted = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x92,
    );
    aborted.initialize().await;
    aborted
        .coordinator()
        .abort(aborted.manifest.manifest_id())
        .await
        .expect("abort");
    assert_eq!(
        aborted.process_at(10).await,
        BackfillProcessOutcomeV1::Aborted
    );

    let excluded = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x94,
    );
    excluded.remove_all_source_data();
    excluded.initialize().await;
    assert!(matches!(
        excluded.process_at(10).await,
        BackfillProcessOutcomeV1::Quarantined {
            failure: BackfillFailureClassV1::MissingSource,
            ..
        }
    ));
    let entry_id = excluded.manifest.entries()[0].entry_id();
    excluded.clock.set(20);
    let approval = FakeApprovalPort::capability();
    let approved = excluded
        .coordinator()
        .approve_disposition(
            excluded.manifest.manifest_id(),
            entry_id,
            BackfillOwnerDispositionV1::ExcludeApproved,
            &approval,
        )
        .await
        .expect("owner exclusion");
    assert_eq!(approved.state(), BackfillRunStateV1::Completed);
}

#[tokio::test]
async fn reference_disposition_reconciles_verified_source_without_a_target() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x95,
    );
    let entry = &fixture.manifest.entries()[0];
    fixture.initialize().await;
    fixture
        .source
        .queue_source_fault(entry, ReadFault::InvalidEmptyChunk);
    assert_eq!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Quarantined {
            entry_id: entry.entry_id(),
            failure: BackfillFailureClassV1::SourceMetadataMismatch,
        }
    );
    fixture.clock.set(20);
    fixture
        .coordinator()
        .approve_disposition(
            fixture.manifest.manifest_id(),
            entry.entry_id(),
            BackfillOwnerDispositionV1::ReferenceApproved,
            &FakeApprovalPort::capability(),
        )
        .await
        .expect("approve reference");

    fixture.clock.set(30);
    let report = fixture
        .coordinator()
        .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
        .await
        .expect("reference reconciliation");
    assert!(report.clean(), "{report:?}");
    assert_eq!(report.expected_source().object_count(), 1);
    assert_eq!(report.source().strong_checksums_verified(), 1);
    assert_eq!(report.expected_target().object_count(), 0);
    assert_eq!(report.target().object_count(), 0);
    assert_eq!(report.dispositions().len(), 1);
    assert_eq!(
        report.dispositions()[0].disposition(),
        BackfillOwnerDispositionV1::ReferenceApproved
    );
    assert_eq!(report.disposition_totals().referenced_objects(), 1);
    assert_eq!(report.disposition_totals().excluded_objects(), 0);
    assert!(report.discrepancies().iter().all(|discrepancy| {
        !matches!(
            discrepancy.kind(),
            BackfillDiscrepancyKindV1::MissingTarget
                | BackfillDiscrepancyKindV1::CheckpointDivergence
        )
    }));

    fixture.clock.set(31);
    let released = fixture
        .coordinator()
        .approve_source_release(
            fixture.manifest.manifest_id(),
            &fixture.bindings(),
            &FakeApprovalPort::capability(),
        )
        .await
        .expect("release referenced source");
    assert!(matches!(
        released.source_retention(),
        BackfillSourceRetentionStateV1::ReleaseApproved { .. }
    ));
}

#[tokio::test]
async fn exclude_disposition_reconciles_missing_and_corrupt_sources_as_audited_exclusions() {
    for (marker, corrupt, expected_failure) in [
        (0x96, false, BackfillFailureClassV1::MissingSource),
        (0x98, true, BackfillFailureClassV1::SourceChecksumMismatch),
    ] {
        let fixture = Fixture::new(
            &[ObjectRole::Source],
            BackfillProviderV1::S3,
            BackfillProviderV1::R2,
            marker,
        );
        let entry = &fixture.manifest.entries()[0];
        if corrupt {
            fixture.source.replace_source_bytes(
                entry,
                vec![
                    b'x';
                    usize::try_from(entry.expected_size().get()).expect("wire-safe object size")
                ],
            );
        } else {
            fixture.source.remove_source(entry);
        }
        fixture.initialize().await;
        assert_eq!(
            fixture.process_at(10).await,
            BackfillProcessOutcomeV1::Quarantined {
                entry_id: entry.entry_id(),
                failure: expected_failure,
            }
        );
        fixture.clock.set(20);
        fixture
            .coordinator()
            .approve_disposition(
                fixture.manifest.manifest_id(),
                entry.entry_id(),
                BackfillOwnerDispositionV1::ExcludeApproved,
                &FakeApprovalPort::capability(),
            )
            .await
            .expect("approve exclusion");

        fixture.clock.set(30);
        let report = fixture
            .coordinator()
            .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
            .await
            .expect("exclusion reconciliation");
        assert!(report.clean(), "{report:?}");
        assert_eq!(report.expected_source().object_count(), 0);
        assert_eq!(report.expected_target().object_count(), 0);
        assert_eq!(report.source().object_count(), 0);
        assert_eq!(report.target().object_count(), 0);
        assert_eq!(report.disposition_totals().referenced_objects(), 0);
        assert_eq!(report.disposition_totals().excluded_objects(), 1);
        assert_eq!(
            report.disposition_totals().excluded_logical_bytes(),
            entry.expected_size().get()
        );
        assert_eq!(
            report
                .disposition_totals()
                .excluded_role_count(ObjectRole::Source),
            1
        );

        fixture.clock.set(31);
        fixture
            .coordinator()
            .approve_source_release(
                fixture.manifest.manifest_id(),
                &fixture.bindings(),
                &FakeApprovalPort::capability(),
            )
            .await
            .expect("release disposition-aware source inventory");
    }
}

#[tokio::test]
async fn fresh_dirty_reconciliation_blocks_source_release() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0x91,
    );
    fixture.initialize().await;
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    fixture.target.replace_target_bytes(
        &fixture.manifest.entries()[0],
        b"corrupt-after-copy".to_vec(),
    );
    fixture.clock.set(30);
    let approval = FakeApprovalPort::capability();
    assert_eq!(
        fixture
            .coordinator()
            .approve_source_release(
                fixture.manifest.manifest_id(),
                &fixture.bindings(),
                &approval,
            )
            .await,
        Err(ObjectBackfillErrorV1::StateConflict)
    );
    assert!(matches!(
        fixture.journal().await.source_retention(),
        BackfillSourceRetentionStateV1::Retained
    ));
}

impl Fixture {
    fn remove_all_source_data(&self) {
        for entry in self.manifest.entries() {
            self.source.remove_source(entry);
        }
    }
}

#[tokio::test]
async fn every_role_and_fake_provider_class_runs_the_streaming_playback_probe() {
    let roles = [
        ObjectRole::Source,
        ObjectRole::RecordingSegment,
        ObjectRole::Thumbnail,
        ObjectRole::Preview,
        ObjectRole::Spritesheet,
        ObjectRole::Audio,
        ObjectRole::Export,
        ObjectRole::Manifest,
    ];
    let fixture = Fixture::new(
        &roles,
        BackfillProviderV1::GoogleDrive,
        BackfillProviderV1::R2,
        0xa0,
    );
    fixture.initialize().await;
    while !matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Completed
    ) {}
    let observed = fixture.probes.roles.lock().expect("roles lock").clone();
    assert_eq!(observed, roles.into_iter().collect());
    fixture.clock.set(30);
    let report = fixture
        .coordinator()
        .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
        .await
        .expect("reconcile all roles");
    assert!(report.clean(), "{report:?}");
}

#[tokio::test]
async fn capability_preflight_and_credential_redaction_fail_closed() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xb0,
    );
    let mut incomplete = full_capabilities();
    incomplete.streaming_conditional_create = false;
    fixture.target.set_capabilities(incomplete);
    assert_eq!(
        fixture
            .coordinator()
            .initialize(&fixture.manifest, &fixture.bindings())
            .await,
        Err(ObjectBackfillErrorV1::CapabilityMissing)
    );
    let debug = format!("{:?}", fixture.bindings());
    assert!(!debug.contains("vault:backfill/source"));
    assert!(!debug.contains("vault:backfill/target"));
    let manifest_debug = format!("{:?}", fixture.manifest);
    assert!(!manifest_debug.contains("vault"));
    assert!(!manifest_debug.contains("credential"));
}

#[tokio::test]
async fn slow_copy_renews_from_fresh_clock_and_throttles_on_both_provider_dimensions() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::GoogleDrive,
        BackfillProviderV1::R2,
        0xb2,
    );
    fixture.initialize().await;
    fixture.clock.set_step(30);
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    assert!(
        fixture.clock.current() > 300,
        "copy must span multiple lease TTLs"
    );
    let observations = fixture.throttle.observations();
    assert!(
        observations
            .iter()
            .any(|observation| observation.bytes.is_none())
    );
    assert!(
        observations
            .iter()
            .any(|observation| observation.bytes.is_some())
    );
    assert!(observations.iter().all(|observation| {
        observation.tenant_id == fixture.tenant()
            && observation.source_provider == BackfillProviderV1::GoogleDrive
            && observation.source_region == fixture.manifest.source().region()
            && observation.target_provider == BackfillProviderV1::R2
            && observation.target_region == fixture.manifest.target().region()
    }));
    assert!(
        observations
            .windows(2)
            .all(|window| window[0].now < window[1].now),
        "every admission must receive a fresh increasing time"
    );
    let journal = fixture.journal().await;
    assert_eq!(journal.usage().admitted_cost_units(), 2);
}

#[tokio::test]
async fn rollback_after_a_durable_transition_is_rejected() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xb3,
    );
    fixture.initialize().await;
    fixture.clock.set(20);
    fixture
        .coordinator()
        .pause(fixture.manifest.manifest_id())
        .await
        .expect("durable transition at time 20");
    fixture.clock.set(19);
    assert_eq!(
        fixture
            .coordinator()
            .resume(fixture.manifest.manifest_id())
            .await,
        Err(ObjectBackfillErrorV1::ClockRollback)
    );
}

#[tokio::test]
async fn live_commit_fence_blocks_abort_and_stale_reclaim_publication() {
    let aborted = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xb4,
    );
    aborted.initialize().await;
    aborted.target.block_commit();
    aborted.clock.set(10);
    let process_coordinator = aborted.coordinator();
    let control_coordinator = aborted.coordinator();
    let process_bindings = aborted.bindings();
    let process = process_coordinator.process_next(
        aborted.manifest.manifest_id(),
        aborted.tenant(),
        &process_bindings,
        BackfillWorkerIdV1::new(),
    );
    let abort = async {
        while !aborted.target.commit_waiting() {
            tokio::task::yield_now().await;
        }
        let journal = control_coordinator
            .abort(aborted.manifest.manifest_id())
            .await
            .expect("persist abort before releasing commit");
        aborted.target.release_commit();
        journal
    };
    let (process_result, aborted_journal) = tokio::join!(process, abort);
    assert_eq!(
        process_result.expect("aborted worker outcome"),
        BackfillProcessOutcomeV1::LeaseLost(aborted.manifest.entries()[0].entry_id())
    );
    assert_eq!(aborted_journal.state(), BackfillRunStateV1::Aborted);
    assert_eq!(aborted.target.create_count(), 0);
    assert!(aborted.target.write_cancel_count() >= 1);
    assert!(aborted.source.read_release_count() >= 1);

    let reclaimed = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::Minio,
        BackfillProviderV1::R2,
        0xb6,
    );
    reclaimed.initialize().await;
    reclaimed.target.block_commit();
    reclaimed.clock.set(10);
    let stale_coordinator = reclaimed.coordinator();
    let reclaim_coordinator = reclaimed.coordinator();
    let stale_bindings = reclaimed.bindings();
    let reclaim_bindings = reclaimed.bindings();
    let stale = stale_coordinator.process_next(
        reclaimed.manifest.manifest_id(),
        reclaimed.tenant(),
        &stale_bindings,
        BackfillWorkerIdV1::new(),
    );
    let reclaim = async {
        while !reclaimed.target.commit_waiting() {
            tokio::task::yield_now().await;
        }
        reclaimed.clock.set(111);
        let outcome = reclaim_coordinator
            .process_next(
                reclaimed.manifest.manifest_id(),
                reclaimed.tenant(),
                &reclaim_bindings,
                BackfillWorkerIdV1::new(),
            )
            .await
            .expect("normalize expired worker");
        reclaimed.target.release_commit();
        outcome
    };
    let (stale_result, reclaim_result) = tokio::join!(stale, reclaim);
    assert!(matches!(
        reclaim_result,
        BackfillProcessOutcomeV1::RetryScheduled {
            failure: BackfillFailureClassV1::TransferCanceled,
            ..
        }
    ));
    assert_eq!(
        stale_result.expect("stale worker outcome"),
        BackfillProcessOutcomeV1::LeaseLost(reclaimed.manifest.entries()[0].entry_id())
    );
    assert_eq!(reclaimed.target.create_count(), 0);
    assert!(matches!(
        reclaimed.process_at(112).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    assert_eq!(reclaimed.target.create_count(), 1);
}

#[tokio::test]
async fn expired_foreign_tenant_half_open_lease_is_reclaimed_before_tenant_selection() {
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let execution_policy = half_open_policy();
    let fixture = Fixture::new_with_tenant_roles(
        &[
            (tenant_a, ObjectRole::Source),
            (tenant_b, ObjectRole::Source),
        ],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xb7,
        execution_policy,
    );
    fixture.initialize().await;
    let entry_a = &fixture.manifest.entries()[0];
    let entry_b = &fixture.manifest.entries()[1];

    let mut journal = fixture.journal().await;
    let revision = journal.revision();
    let first_lease = journal
        .claim(
            entry_a.entry_id(),
            BackfillWorkerIdV1::new(),
            BackfillOperationIdV1::new(),
            timestamp(10),
            execution_policy,
            entry_a.expected_size(),
            2,
        )
        .expect("claim tenant A before opening circuit");
    fixture
        .journals
        .compare_and_swap(fixture.manifest.manifest_id(), revision, &journal)
        .await
        .expect("persist initial tenant A lease");

    let mut journal = fixture.journal().await;
    let revision = journal.revision();
    journal
        .fail(
            entry_a.entry_id(),
            first_lease,
            BackfillFailureClassV1::ProviderOutage,
            timestamp(11),
            execution_policy,
        )
        .expect("open circuit");
    fixture
        .journals
        .compare_and_swap(fixture.manifest.manifest_id(), revision, &journal)
        .await
        .expect("persist open circuit");

    let mut journal = fixture.journal().await;
    let revision = journal.revision();
    let half_open_lease = journal
        .claim(
            entry_a.entry_id(),
            BackfillWorkerIdV1::new(),
            BackfillOperationIdV1::new(),
            timestamp(1_011),
            execution_policy,
            entry_a.expected_size(),
            2,
        )
        .expect("claim the sole half-open probe");
    fixture
        .journals
        .compare_and_swap(fixture.manifest.manifest_id(), revision, &journal)
        .await
        .expect("persist half-open lease");

    let journal = fixture.journal().await;
    assert_eq!(
        journal.circuit().half_open_fencing_token(),
        Some(half_open_lease.fencing_token())
    );
    assert_eq!(
        journal
            .entries()
            .iter()
            .filter(|progress| matches!(
                progress.status(),
                BackfillEntryStatusV1::Leased { lease, .. }
                    if !lease.expired_at(timestamp(1_012))
            ))
            .count(),
        1
    );
    let mut second_probe = journal.clone();
    assert_eq!(
        second_probe.claim(
            entry_b.entry_id(),
            BackfillWorkerIdV1::new(),
            BackfillOperationIdV1::new(),
            timestamp(1_012),
            execution_policy,
            entry_b.expected_size(),
            2,
        ),
        Err(BackfillContractErrorV1::InvalidTransition)
    );

    fixture.clock.set(1_112);
    let outcome = fixture
        .coordinator()
        .process_next(
            fixture.manifest.manifest_id(),
            tenant_b,
            &fixture.bindings(),
            BackfillWorkerIdV1::new(),
        )
        .await
        .expect("globally reclaim tenant A, then process tenant B");
    assert_eq!(
        outcome,
        BackfillProcessOutcomeV1::Copied(entry_b.entry_id())
    );

    let journal = fixture.journal().await;
    assert!(matches!(
        journal
            .entry(entry_a.entry_id())
            .expect("tenant A progress")
            .status(),
        BackfillEntryStatusV1::RetryScheduled {
            failure: BackfillFailureClassV1::TransferCanceled,
            ..
        }
    ));
    assert!(matches!(
        journal
            .entry(entry_b.entry_id())
            .expect("tenant B progress")
            .status(),
        BackfillEntryStatusV1::Succeeded { .. }
    ));
    assert_eq!(journal.circuit().half_open_fencing_token(), None);
    assert_eq!(journal.circuit().open_until(), None);
}

#[tokio::test]
async fn approval_capabilities_are_trusted_expiring_and_not_caller_asserted() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xb8,
    );
    fixture.remove_all_source_data();
    fixture.initialize().await;
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Quarantined {
            failure: BackfillFailureClassV1::MissingSource,
            ..
        }
    ));
    let entry_id = fixture.manifest.entries()[0].entry_id();
    let forged = BackfillOwnerApprovalCapabilityV1::parse("forged-owner-capability-v1")
        .expect("opaque forged capability");
    fixture.clock.set(20);
    assert_eq!(
        fixture
            .coordinator()
            .approve_disposition(
                fixture.manifest.manifest_id(),
                entry_id,
                BackfillOwnerDispositionV1::ExcludeApproved,
                &forged,
            )
            .await,
        Err(ObjectBackfillErrorV1::ApprovalDenied)
    );
    fixture.approvals.expired.store(true, Ordering::SeqCst);
    let expired = FakeApprovalPort::capability();
    fixture.clock.set(21);
    assert_eq!(
        fixture
            .coordinator()
            .approve_disposition(
                fixture.manifest.manifest_id(),
                entry_id,
                BackfillOwnerDispositionV1::ExcludeApproved,
                &expired,
            )
            .await,
        Err(ObjectBackfillErrorV1::ApprovalDenied)
    );
    assert!(matches!(
        fixture.journal().await.entries()[0].status(),
        BackfillEntryStatusV1::Quarantined {
            disposition: BackfillOwnerDispositionV1::PendingOwnerApproval,
            ..
        }
    ));
}

#[tokio::test]
async fn target_head_then_open_errors_remain_target_specific() {
    let target_cases = [
        (
            BackfillPortErrorV1::NotFound,
            BackfillFailureClassV1::MissingTarget,
        ),
        (
            BackfillPortErrorV1::InvalidResponse,
            BackfillFailureClassV1::TargetMetadataMismatch,
        ),
    ];
    for (index, (port_error, expected_failure)) in target_cases.into_iter().enumerate() {
        let fixture = Fixture::new_with_policy(
            &[ObjectRole::Source],
            BackfillProviderV1::S3,
            BackfillProviderV1::R2,
            0xba + u8::try_from(index).expect("marker"),
            bounded_policy(1, 1, 10, 10_000, 10),
        );
        let entry = &fixture.manifest.entries()[0];
        let bytes = b"valid-media-source-0-payload".to_vec();
        fixture.target.seed_target(entry, bytes);
        fixture
            .target
            .queue_target_fault(entry, ReadFault::Open(port_error));
        fixture.initialize().await;
        assert_eq!(
            fixture.process_at(10).await,
            BackfillProcessOutcomeV1::Quarantined {
                entry_id: entry.entry_id(),
                failure: expected_failure,
            }
        );
    }

    let source = Fixture::new_with_policy(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xbc,
        bounded_policy(1, 1, 10, 10_000, 10),
    );
    source.source.queue_source_fault(
        &source.manifest.entries()[0],
        ReadFault::Open(BackfillPortErrorV1::NotFound),
    );
    source.initialize().await;
    assert!(matches!(
        source.process_at(10).await,
        BackfillProcessOutcomeV1::Quarantined {
            failure: BackfillFailureClassV1::MissingSource,
            ..
        }
    ));
}

#[tokio::test]
async fn reconciliation_detects_created_receipt_operation_provenance_drift() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xbd,
    );
    fixture.initialize().await;
    assert!(matches!(
        fixture.process_at(10).await,
        BackfillProcessOutcomeV1::Copied(_)
    ));
    fixture
        .target
        .replace_target_operation(&fixture.manifest.entries()[0], BackfillOperationIdV1::new());
    fixture.clock.set(30);
    let report = fixture
        .coordinator()
        .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
        .await
        .expect("reconcile provenance drift");
    assert!(report.discrepancies().iter().any(|discrepancy| {
        discrepancy.side() == BackfillInventorySideV1::Target
            && discrepancy.kind() == BackfillDiscrepancyKindV1::CheckpointDivergence
    }));
    report
        .validate_integrity(
            &fixture.manifest,
            timestamp(30),
            DurationMillis::new(300_000).expect("report age"),
        )
        .expect("report is internally authentic despite observed drift");
}

#[tokio::test]
async fn inventory_pagination_over_one_page_is_complete_and_tenant_isolated() {
    let roles = vec![ObjectRole::Source; 501];
    let fixture = Fixture::new(&roles, BackfillProviderV1::S3, BackfillProviderV1::R2, 0xbe);
    let foreign_bytes = b"valid-foreign-object".to_vec();
    let foreign = manifest_entry(TenantId::new(), ObjectRole::Export, 9_999, &foreign_bytes);
    fixture.source.seed_source(&foreign, foreign_bytes);
    fixture.initialize().await;
    fixture.clock.set(30);
    let report = fixture
        .coordinator()
        .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
        .await
        .expect("reconcile paginated inventory");
    assert_eq!(report.source().object_count(), 501);
    assert_eq!(report.expected_source().object_count(), 501);
    assert!(!report.discrepancies().iter().any(|discrepancy| {
        discrepancy.side() == BackfillInventorySideV1::Source
            && discrepancy.kind() == BackfillDiscrepancyKindV1::ProviderUnavailable
    }));
    report
        .validate_integrity(
            &fixture.manifest,
            timestamp(30),
            DurationMillis::new(300_000).expect("report age"),
        )
        .expect("complete paginated report");
}

#[tokio::test]
async fn inventory_snapshot_cursor_page_and_scope_faults_fail_closed() {
    let simple_faults = [
        InventoryFault::SnapshotMutation,
        InventoryFault::PageIndex(7),
        InventoryFault::RepeatCursor,
        InventoryFault::EmptyPageWithNext,
    ];
    for (index, fault) in simple_faults.into_iter().enumerate() {
        let fixture = Fixture::new(
            &[ObjectRole::Source],
            BackfillProviderV1::S3,
            BackfillProviderV1::R2,
            0xc2 + u8::try_from(index).expect("marker"),
        );
        fixture.initialize().await;
        fixture.source.set_inventory_fault(fault);
        fixture.clock.set(30);
        let report = fixture
            .coordinator()
            .reconcile(fixture.manifest.manifest_id(), &fixture.bindings())
            .await
            .expect("fault is represented as an unavailable inventory");
        assert!(report.discrepancies().iter().any(|discrepancy| {
            discrepancy.side() == BackfillInventorySideV1::Source
                && discrepancy.kind() == BackfillDiscrepancyKindV1::ProviderUnavailable
        }));
    }

    let scoped = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::S3,
        BackfillProviderV1::R2,
        0xc6,
    );
    let foreign_bytes = b"foreign-row".to_vec();
    let foreign = manifest_entry(TenantId::new(), ObjectRole::Export, 10_001, &foreign_bytes);
    scoped.source.seed_source(&foreign, foreign_bytes);
    let foreign_metadata = scoped.source.metadata_for_source(&foreign);
    scoped.initialize().await;
    scoped
        .source
        .set_inventory_fault(InventoryFault::CrossTenant(Box::new(foreign_metadata)));
    scoped.clock.set(30);
    let report = scoped
        .coordinator()
        .reconcile(scoped.manifest.manifest_id(), &scoped.bindings())
        .await
        .expect("cross-tenant inventory row fails closed");
    assert!(report.discrepancies().iter().any(|discrepancy| {
        discrepancy.side() == BackfillInventorySideV1::Source
            && discrepancy.kind() == BackfillDiscrepancyKindV1::ProviderUnavailable
    }));
}

#[tokio::test]
async fn unplayable_media_is_quarantined_and_never_published() {
    let fixture = Fixture::new(
        &[ObjectRole::Source],
        BackfillProviderV1::Minio,
        BackfillProviderV1::R2,
        0xc0,
    );
    let entry = &fixture.manifest.entries()[0];
    let bytes = b"CORRUPT-media-payload".to_vec();
    // Build a separate exact manifest so SHA succeeds and the streaming probe is decisive.
    let bad_entry = manifest_entry(entry.tenant_id(), entry.role(), 77, &bytes);
    let source = FakeProvider::new(fixture.manifest.source().clone());
    source.seed_source(&bad_entry, bytes);
    let target = FakeProvider::new(fixture.manifest.target().clone());
    let manifest = ObjectBackfillManifestV1::new(
        BackfillManifestIdV1::new(),
        timestamp(1),
        "probe-rehearsal",
        "code-probe",
        fixture.manifest.source().clone(),
        fixture.manifest.target().clone(),
        policy(3),
        vec![bad_entry.clone()],
    )
    .expect("probe manifest");
    let manifests = MemoryBackfillManifestPortV1::default();
    let journals = MemoryBackfillJournalPortV1::default();
    let probes = FakeProbePort::default();
    let throttle = FakeThrottle::default();
    let cancellation = FakeCancellation::default();
    let clock = FakeClock::new(10);
    let approvals = FakeApprovalPort::default();
    let coordinator = ObjectBackfillCoordinatorV1::new(
        &manifests,
        &journals,
        &source,
        &target,
        &probes,
        &throttle,
        &cancellation,
        &clock,
        &approvals,
    );
    let bindings = BackfillRuntimeBindingsV1::new(
        BackfillProviderAccessV1::new(manifest.source(), &fixture.source_credential),
        BackfillProviderAccessV1::new(manifest.target(), &fixture.target_credential),
    );
    coordinator
        .initialize(&manifest, &bindings)
        .await
        .expect("initialize probe manifest");
    assert_eq!(
        coordinator
            .process_next(
                manifest.manifest_id(),
                bad_entry.tenant_id(),
                &bindings,
                BackfillWorkerIdV1::new(),
            )
            .await
            .expect("process"),
        BackfillProcessOutcomeV1::Quarantined {
            entry_id: bad_entry.entry_id(),
            failure: BackfillFailureClassV1::MediaUnplayable,
        }
    );
    assert_eq!(target.create_count(), 0);
}
