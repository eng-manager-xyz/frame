use std::{collections::BTreeMap, fmt};

use frame_domain::{
    ByteSize, ChecksumSha256, CorsOriginV1, DownloadDispositionV1, IdempotencyKey,
    MultipartGrantId, MultipartGrantKeyVersion, MultipartGrantRecordV1, MultipartGrantScopeV1,
    MultipartGrantSecret, MultipartLimitsV1, MultipartOperationV1, MultipartPartNumberV1,
    MultipartUploadId, MultipartUploadSpecV1, ScopedObjectKey, SecretDigest, TimestampMillis,
};
use frame_ports::{
    DownloadPolicyV1, DownloadValidatorV1, JournalCreateOutcomeV1, JournalMutationOutcomeV1,
    MultipartJournalPhaseV1, MultipartJournalV1, MultipartObjectStoreV1,
    MultipartProviderOperationV1, MultipartReplayKeyV1, MultipartSystemReplayOperationV1,
    MultipartUploadSnapshotV1, ObjectByteRange, ObjectCachePolicy, ProviderAbortDispositionV1,
    ProviderAbortReceiptV1, ProviderCompleteMultipartRequestV1, ProviderCompletedObjectV1,
    ProviderCreateMultipartRequestV1, ProviderDownloadBodyV1, ProviderDownloadMetadataV1,
    ProviderDownloadRequestV1, ProviderDownloadResponseV1, ProviderEntityTag,
    ProviderLookupMultipartRequestV1, ProviderMultipartHandleV1, ProviderMultipartSessionV1,
    ProviderPartReceiptV1, ProviderPartsListV1, ProviderPutPartRequestV1,
    ProviderUploadReferenceV1, SourceFinalizeRecordV1, StorageFailure, StorageFailureKind,
    StorageRequestContext,
};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq)]
pub struct MultipartGrantKeyMaterialV1(Vec<u8>);

impl MultipartGrantKeyMaterialV1 {
    pub fn parse(value: impl Into<Vec<u8>>) -> Result<Self, StorageFailure> {
        let value = value.into();
        if !(32..=64).contains(&value.len()) {
            return Err(invalid());
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for MultipartGrantKeyMaterialV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MultipartGrantKeyMaterialV1([redacted])")
    }
}

/// Server-side verification keys. Retaining an old version permits bounded rotation overlap;
/// removing it revokes every still-unexpired grant made with that version.
pub struct MultipartGrantKeyRingV1 {
    active: MultipartGrantKeyVersion,
    keys: BTreeMap<MultipartGrantKeyVersion, MultipartGrantKeyMaterialV1>,
}

impl MultipartGrantKeyRingV1 {
    pub fn new(
        active: MultipartGrantKeyVersion,
        keys: impl IntoIterator<Item = (MultipartGrantKeyVersion, MultipartGrantKeyMaterialV1)>,
    ) -> Result<Self, StorageFailure> {
        let mut unique = BTreeMap::new();
        for (version, key) in keys {
            if unique.insert(version, key).is_some() {
                return Err(invalid());
            }
        }
        let keys = unique;
        if keys.is_empty() || !keys.contains_key(&active) || keys.len() > 8 {
            return Err(invalid());
        }
        Ok(Self { active, keys })
    }

    #[must_use]
    pub const fn active_version(&self) -> MultipartGrantKeyVersion {
        self.active
    }

    fn digest(
        &self,
        version: MultipartGrantKeyVersion,
        secret: &MultipartGrantSecret,
    ) -> Option<SecretDigest> {
        let key = self.keys.get(&version)?;
        multipart_grant_digest(
            &key.0,
            b"frame.multipart.grant.v1",
            version,
            secret.expose_for_hashing(),
        )
    }
}

impl fmt::Debug for MultipartGrantKeyRingV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MultipartGrantKeyRingV1")
            .field("active", &self.active)
            .field("key_count", &self.keys.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MultipartAuthorizationV1 {
    grant_id: MultipartGrantId,
    grant_secret: MultipartGrantSecret,
}

impl MultipartAuthorizationV1 {
    #[must_use]
    pub const fn new(grant_id: MultipartGrantId, grant_secret: MultipartGrantSecret) -> Self {
        Self {
            grant_id,
            grant_secret,
        }
    }

    #[must_use]
    pub const fn grant_id(&self) -> MultipartGrantId {
        self.grant_id
    }
}

impl fmt::Debug for MultipartAuthorizationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MultipartAuthorizationV1")
            .field("grant_id", &self.grant_id)
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartUploadPlanV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    handle: ProviderMultipartHandleV1,
    part_size: ByteSize,
    part_count: u16,
    total_size: ByteSize,
    expires_at: TimestampMillis,
    correlation_id: frame_domain::CorrelationId,
}

impl MultipartUploadPlanV1 {
    fn from_session(spec: &MultipartUploadSpecV1, session: &ProviderMultipartSessionV1) -> Self {
        Self {
            upload_id: session.upload_id(),
            key: session.key().clone(),
            handle: session.handle().clone(),
            part_size: spec.part_size(),
            part_count: spec.part_count(),
            total_size: spec.total_size(),
            expires_at: session.expires_at(),
            correlation_id: session.correlation_id(),
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn handle(&self) -> &ProviderMultipartHandleV1 {
        &self.handle
    }

    #[must_use]
    pub const fn part_size(&self) -> ByteSize {
        self.part_size
    }

    #[must_use]
    pub const fn part_count(&self) -> u16 {
        self.part_count
    }

    #[must_use]
    pub const fn total_size(&self) -> ByteSize {
        self.total_size
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn correlation_id(&self) -> frame_domain::CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultipartRetryDirectiveV1 {
    PersistJournalBeforeTransfer,
    RetrySamePartAndIdempotencyKey,
    ListVerifiedPartsAfterRestart,
    RestartAfterExpiry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartResumePlanV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    verified_parts: Vec<ProviderPartReceiptV1>,
    retry_directives: [MultipartRetryDirectiveV1; 4],
    expires_at: TimestampMillis,
    correlation_id: frame_domain::CorrelationId,
}

impl MultipartResumePlanV1 {
    #[must_use]
    pub fn verified_parts(&self) -> &[ProviderPartReceiptV1] {
        &self.verified_parts
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn retry_directives(&self) -> &[MultipartRetryDirectiveV1; 4] {
        &self.retry_directives
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn correlation_id(&self) -> frame_domain::CorrelationId {
        self.correlation_id
    }
}

pub struct CreateMultipartCommandV1 {
    authorization: MultipartAuthorizationV1,
    idempotency_key: IdempotencyKey,
    spec: MultipartUploadSpecV1,
    expires_at: TimestampMillis,
    now: TimestampMillis,
}

impl CreateMultipartCommandV1 {
    #[must_use]
    pub const fn new(
        authorization: MultipartAuthorizationV1,
        idempotency_key: IdempotencyKey,
        spec: MultipartUploadSpecV1,
        expires_at: TimestampMillis,
        now: TimestampMillis,
    ) -> Self {
        Self {
            authorization,
            idempotency_key,
            spec,
            expires_at,
            now,
        }
    }
}

impl fmt::Debug for CreateMultipartCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateMultipartCommandV1")
            .field("authorization", &self.authorization)
            .field("idempotency_key", &self.idempotency_key)
            .field("spec", &self.spec)
            .field("expires_at", &self.expires_at)
            .field("now", &self.now)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ScopedMultipartCommandV1 {
    authorization: MultipartAuthorizationV1,
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    now: TimestampMillis,
}

impl ScopedMultipartCommandV1 {
    #[must_use]
    pub const fn new(
        authorization: MultipartAuthorizationV1,
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        now: TimestampMillis,
    ) -> Self {
        Self {
            authorization,
            upload_id,
            key,
            now,
        }
    }
}

pub struct PutMultipartPartCommandV1 {
    scoped: ScopedMultipartCommandV1,
    idempotency_key: IdempotencyKey,
    part_number: MultipartPartNumberV1,
    checksum_sha256: ChecksumSha256,
    bytes: Vec<u8>,
}

impl PutMultipartPartCommandV1 {
    #[must_use]
    pub fn new(
        scoped: ScopedMultipartCommandV1,
        idempotency_key: IdempotencyKey,
        part_number: MultipartPartNumberV1,
        checksum_sha256: ChecksumSha256,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            scoped,
            idempotency_key,
            part_number,
            checksum_sha256,
            bytes,
        }
    }
}

impl fmt::Debug for PutMultipartPartCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PutMultipartPartCommandV1")
            .field("scoped", &self.scoped)
            .field("idempotency_key", &self.idempotency_key)
            .field("part_number", &self.part_number)
            .field("checksum_sha256", &self.checksum_sha256)
            .field("byte_length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct IdempotentMultipartCommandV1 {
    scoped: ScopedMultipartCommandV1,
    idempotency_key: IdempotencyKey,
}

impl IdempotentMultipartCommandV1 {
    #[must_use]
    pub const fn new(scoped: ScopedMultipartCommandV1, idempotency_key: IdempotencyKey) -> Self {
        Self {
            scoped,
            idempotency_key,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateDownloadMethodV1 {
    Head,
    Get,
}

#[derive(Debug, Clone)]
pub struct PrivateDownloadCommandV1 {
    authorization: MultipartAuthorizationV1,
    key: ScopedObjectKey,
    method: PrivateDownloadMethodV1,
    range: Option<ObjectByteRange>,
    validator: DownloadValidatorV1,
    origin: Option<CorsOriginV1>,
    disposition: DownloadDispositionV1,
    now: TimestampMillis,
}

impl PrivateDownloadCommandV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        authorization: MultipartAuthorizationV1,
        key: ScopedObjectKey,
        method: PrivateDownloadMethodV1,
        range: Option<ObjectByteRange>,
        validator: DownloadValidatorV1,
        origin: Option<CorsOriginV1>,
        disposition: DownloadDispositionV1,
        now: TimestampMillis,
    ) -> Self {
        Self {
            authorization,
            key,
            method,
            range,
            validator,
            origin,
            disposition,
            now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateDownloadStatusV1 {
    Ok,
    PartialContent,
    NotModified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateDownloadContentRangeV1 {
    range: ObjectByteRange,
    total_size: ByteSize,
}

impl PrivateDownloadContentRangeV1 {
    #[must_use]
    pub const fn new(range: ObjectByteRange, total_size: ByteSize) -> Self {
        Self { range, total_size }
    }

    #[must_use]
    pub const fn range(self) -> ObjectByteRange {
        self.range
    }

    #[must_use]
    pub const fn total_size(self) -> ByteSize {
        self.total_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateDownloadHeadersV1 {
    content_length: Option<ByteSize>,
    content_range: Option<PrivateDownloadContentRangeV1>,
    content_type: frame_domain::ContentType,
    disposition: DownloadDispositionV1,
    etag: ProviderEntityTag,
    last_modified: TimestampMillis,
    cache_policy: ObjectCachePolicy,
    cors_allow_origin: Option<CorsOriginV1>,
    vary_origin: bool,
    accept_ranges: bool,
}

impl PrivateDownloadHeadersV1 {
    #[must_use]
    pub const fn content_length(&self) -> Option<ByteSize> {
        self.content_length
    }

    #[must_use]
    pub const fn content_range(&self) -> Option<PrivateDownloadContentRangeV1> {
        self.content_range
    }

    #[must_use]
    pub const fn content_type(&self) -> &frame_domain::ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn disposition(&self) -> DownloadDispositionV1 {
        self.disposition
    }

    #[must_use]
    pub const fn etag(&self) -> &ProviderEntityTag {
        &self.etag
    }

    #[must_use]
    pub const fn last_modified(&self) -> TimestampMillis {
        self.last_modified
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }

    #[must_use]
    pub const fn cors_allow_origin(&self) -> Option<&CorsOriginV1> {
        self.cors_allow_origin.as_ref()
    }

    #[must_use]
    pub const fn vary_origin(&self) -> bool {
        self.vary_origin
    }

    #[must_use]
    pub const fn accept_ranges(&self) -> bool {
        self.accept_ranges
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateDownloadBodyStateV1 {
    Active,
    Complete,
    Failed,
    Cancelled,
}

pub struct PrivateDownloadBodyV1 {
    provider: Option<Box<dyn ProviderDownloadBodyV1>>,
    expected_length: u64,
    maximum_chunk_size: u64,
    consumed: u64,
    digest: Option<Sha256>,
    expected_checksum: Option<ChecksumSha256>,
    state: PrivateDownloadBodyStateV1,
}

impl PrivateDownloadBodyV1 {
    fn new(
        provider: Box<dyn ProviderDownloadBodyV1>,
        expected_length: ByteSize,
        maximum_chunk_size: ByteSize,
        expected_checksum: Option<ChecksumSha256>,
    ) -> Self {
        Self {
            provider: Some(provider),
            expected_length: expected_length.get(),
            maximum_chunk_size: maximum_chunk_size.get(),
            consumed: 0,
            digest: expected_checksum.as_ref().map(|_| Sha256::new()),
            expected_checksum,
            state: PrivateDownloadBodyStateV1::Active,
        }
    }

    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure> {
        match self.state {
            PrivateDownloadBodyStateV1::Complete | PrivateDownloadBodyStateV1::Cancelled => {
                return Ok(None);
            }
            PrivateDownloadBodyStateV1::Failed => return Err(integrity()),
            PrivateDownloadBodyStateV1::Active => {}
        }
        let next = self
            .provider
            .as_mut()
            .ok_or_else(integrity)?
            .next_chunk()
            .await;
        let chunk = match next {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return self.fail_integrity(),
            Err(error) => {
                self.provider.take();
                self.state = PrivateDownloadBodyStateV1::Failed;
                return Err(error);
            }
        };
        let chunk_length = u64::try_from(chunk.len()).map_err(|_| integrity())?;
        if chunk_length == 0 || chunk_length > self.maximum_chunk_size {
            return self.fail_integrity();
        }
        let consumed = self
            .consumed
            .checked_add(chunk_length)
            .ok_or_else(integrity)?;
        if consumed > self.expected_length {
            return self.fail_integrity();
        }
        if let Some(digest) = &mut self.digest {
            digest.update(&chunk);
        }
        self.consumed = consumed;
        if consumed == self.expected_length {
            let terminal = self
                .provider
                .as_mut()
                .ok_or_else(integrity)?
                .next_chunk()
                .await;
            match terminal {
                Ok(None) => {}
                Ok(Some(_)) => return self.fail_integrity(),
                Err(error) => {
                    self.provider.take();
                    self.state = PrivateDownloadBodyStateV1::Failed;
                    return Err(error);
                }
            }
            if let (Some(digest), Some(expected)) = (&self.digest, &self.expected_checksum) {
                let actual = hex_lower(&digest.clone().finalize());
                if actual != expected.as_str() {
                    return self.fail_integrity();
                }
            }
            self.provider.take();
            self.state = PrivateDownloadBodyStateV1::Complete;
        }
        Ok(Some(chunk))
    }

    pub fn cancel(&mut self) {
        self.provider.take();
        self.state = PrivateDownloadBodyStateV1::Cancelled;
    }

    #[must_use]
    pub const fn consumed(&self) -> u64 {
        self.consumed
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.state, PrivateDownloadBodyStateV1::Complete)
    }

    fn fail_integrity<T>(&mut self) -> Result<T, StorageFailure> {
        self.provider.take();
        self.state = PrivateDownloadBodyStateV1::Failed;
        Err(integrity())
    }
}

impl fmt::Debug for PrivateDownloadBodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateDownloadBodyV1")
            .field("expected_length", &self.expected_length)
            .field("maximum_chunk_size", &self.maximum_chunk_size)
            .field("consumed", &self.consumed)
            .field("full_checksum", &self.expected_checksum.is_some())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Drop for PrivateDownloadBodyV1 {
    fn drop(&mut self) {
        self.provider.take();
    }
}

pub struct PrivateDownloadResponseV1 {
    status: PrivateDownloadStatusV1,
    headers: PrivateDownloadHeadersV1,
    body: Option<PrivateDownloadBodyV1>,
    correlation_id: frame_domain::CorrelationId,
}

impl PrivateDownloadResponseV1 {
    #[must_use]
    pub const fn status(&self) -> PrivateDownloadStatusV1 {
        self.status
    }

    #[must_use]
    pub const fn headers(&self) -> &PrivateDownloadHeadersV1 {
        &self.headers
    }

    #[must_use]
    pub fn body_mut(&mut self) -> Option<&mut PrivateDownloadBodyV1> {
        self.body.as_mut()
    }

    #[must_use]
    pub const fn has_body(&self) -> bool {
        self.body.is_some()
    }

    #[must_use]
    pub fn take_body(&mut self) -> Option<PrivateDownloadBodyV1> {
        self.body.take()
    }

    #[must_use]
    pub const fn correlation_id(&self) -> frame_domain::CorrelationId {
        self.correlation_id
    }
}

impl fmt::Debug for PrivateDownloadResponseV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateDownloadResponseV1")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .field("correlation_id", &self.correlation_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipartReconcileOutcomeV1 {
    Activated(MultipartUploadPlanV1),
    WaitingForParts(MultipartResumePlanV1),
    ProviderCompleted(ProviderCompletedObjectV1),
    Finalized(SourceFinalizeRecordV1),
    Aborted(ProviderAbortReceiptV1),
    AlreadyTerminal(MultipartJournalPhaseV1),
}

pub struct MultipartStorageServiceV1<'a> {
    provider: &'a dyn MultipartObjectStoreV1,
    journal: &'a dyn MultipartJournalV1,
    keys: MultipartGrantKeyRingV1,
    limits: MultipartLimitsV1,
    download_policy: DownloadPolicyV1,
}

impl<'a> MultipartStorageServiceV1<'a> {
    #[must_use]
    pub const fn new(
        provider: &'a dyn MultipartObjectStoreV1,
        journal: &'a dyn MultipartJournalV1,
        keys: MultipartGrantKeyRingV1,
        limits: MultipartLimitsV1,
        download_policy: DownloadPolicyV1,
    ) -> Self {
        Self {
            provider,
            journal,
            keys,
            limits,
            download_policy,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn register_grant(
        &self,
        context: StorageRequestContext,
        grant_id: MultipartGrantId,
        secret: &MultipartGrantSecret,
        key_version: MultipartGrantKeyVersion,
        scope: MultipartGrantScopeV1,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
    ) -> Result<MultipartGrantRecordV1, StorageFailure> {
        if scope.tenant_id() != context.tenant_id()
            || scope.key().tenant_id() != context.tenant_id()
            || key_version != self.keys.active_version()
        {
            return Err(not_found());
        }
        let ttl = expires_at
            .get()
            .checked_sub(issued_at.get())
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(invalid)?;
        if ttl == 0 || ttl > self.limits.max_grant_ttl().get() {
            return Err(invalid());
        }
        let digest = self.keys.digest(key_version, secret).ok_or_else(invalid)?;
        let record = MultipartGrantRecordV1::active(
            grant_id,
            digest,
            key_version,
            scope,
            issued_at,
            expires_at,
        )
        .map_err(|_| invalid())?;
        let expected = record.clone();
        let returned = match self.journal.register_grant(context, record).await? {
            JournalMutationOutcomeV1::Applied(record)
            | JournalMutationOutcomeV1::Replay(record) => record,
        };
        if returned != expected {
            return Err(integrity());
        }
        Ok(returned)
    }

    pub async fn revoke_grant(
        &self,
        context: StorageRequestContext,
        grant_id: MultipartGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        self.journal
            .revoke_grant(context, grant_id, revoked_at)
            .await
    }

    pub async fn create(
        &self,
        context: StorageRequestContext,
        command: CreateMultipartCommandV1,
    ) -> Result<MultipartUploadPlanV1, StorageFailure> {
        let grant = self
            .authorize(
                context,
                &command.authorization,
                command.spec.key(),
                None,
                MultipartOperationV1::Create,
                command.now,
            )
            .await?;
        if command.expires_at <= command.now || command.expires_at > grant.expires_at() {
            return Err(not_found());
        }
        self.preflight_spec(&command.spec)?;
        let fingerprint = create_fingerprint(&command.spec, command.expires_at);
        let draft = MultipartUploadSnapshotV1::new(
            MultipartUploadId::new(),
            command.spec.clone(),
            None,
            Vec::new(),
            MultipartJournalPhaseV1::Creating,
            None,
            command.expires_at,
            context.correlation_id(),
        );
        let outcome = self
            .journal
            .claim_create(
                context,
                grant.id(),
                command.now,
                command.idempotency_key,
                fingerprint,
                draft,
            )
            .await?;
        let snapshot = match outcome {
            JournalCreateOutcomeV1::Replay(snapshot) if snapshot.provider_session().is_some() => {
                validate_snapshot_structure(context, &snapshot)?;
                if snapshot.spec() != &command.spec || snapshot.expires_at() != command.expires_at {
                    return Err(integrity());
                }
                return self.plan_from_snapshot(context, &snapshot);
            }
            JournalCreateOutcomeV1::Replay(_) => return Err(integrity()),
            JournalCreateOutcomeV1::Claimed(snapshot)
            | JournalCreateOutcomeV1::Resume(snapshot) => snapshot,
        };
        validate_snapshot_structure(context, &snapshot)?;
        if snapshot.phase() != MultipartJournalPhaseV1::Creating
            || snapshot.spec() != &command.spec
            || snapshot.expires_at() != command.expires_at
        {
            return Err(integrity());
        }
        let session = self.create_or_recover_session(context, &snapshot).await?;
        let activated = self
            .journal
            .activate_upload(context, session.clone())
            .await?;
        validate_snapshot_structure(context, &activated)?;
        if activated.phase() != MultipartJournalPhaseV1::Uploading
            || activated.upload_id() != session.upload_id()
            || activated
                .provider_session()
                .map(ProviderMultipartSessionV1::handle)
                != Some(session.handle())
        {
            return Err(integrity());
        }
        Ok(MultipartUploadPlanV1::from_session(
            snapshot.spec(),
            &session,
        ))
    }

    pub async fn list_parts(
        &self,
        context: StorageRequestContext,
        command: ScopedMultipartCommandV1,
    ) -> Result<MultipartResumePlanV1, StorageFailure> {
        self.authorize_scoped(context, &command, MultipartOperationV1::ListParts)
            .await?;
        let snapshot = self.load_active(context, &command).await?;
        let list = self
            .provider
            .list_parts(context, provider_reference(context, &snapshot)?)
            .await?;
        self.resume_plan(context, &snapshot, list)
    }

    pub async fn put_part(
        &self,
        context: StorageRequestContext,
        command: PutMultipartPartCommandV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        self.authorize_scoped(context, &command.scoped, MultipartOperationV1::PutPart)
            .await?;
        let snapshot = self.load_active(context, &command.scoped).await?;
        let reference = provider_reference(context, &snapshot)?;
        let size = ByteSize::new(u64::try_from(command.bytes.len()).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
        snapshot
            .spec()
            .validate_part(command.part_number, size)
            .map_err(|_| invalid())?;
        if ChecksumSha256::digest_bytes(&command.bytes) != command.checksum_sha256 {
            return Err(integrity());
        }
        let fingerprint = part_fingerprint(
            snapshot.upload_id(),
            snapshot.spec().key(),
            command.part_number,
            size,
            &command.checksum_sha256,
        );
        let request = ProviderPutPartRequestV1::new(
            reference,
            command.part_number,
            command.checksum_sha256.clone(),
            command.bytes,
        );
        let receipt = self.provider.put_part(context, request).await?;
        validate_part_receipt(
            context,
            &snapshot,
            command.part_number,
            size,
            &command.checksum_sha256,
            &receipt,
        )?;
        let expected = receipt.clone();
        let returned = match self
            .journal
            .record_part(
                context,
                MultipartReplayKeyV1::client(command.idempotency_key),
                fingerprint,
                receipt,
            )
            .await?
        {
            JournalMutationOutcomeV1::Applied(receipt)
            | JournalMutationOutcomeV1::Replay(receipt) => receipt,
        };
        if returned != expected {
            return Err(integrity());
        }
        validate_part_receipt(
            context,
            &snapshot,
            command.part_number,
            size,
            &command.checksum_sha256,
            &returned,
        )?;
        Ok(returned)
    }

    pub async fn complete(
        &self,
        context: StorageRequestContext,
        command: IdempotentMultipartCommandV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        self.authorize_scoped(context, &command.scoped, MultipartOperationV1::Complete)
            .await?;
        let snapshot = self
            .load_snapshot(context, command.scoped.upload_id, &command.scoped.key)
            .await?;
        let fingerprint = complete_fingerprint(&snapshot);
        let completed = if let Some(completed) = snapshot.completed() {
            rebind_completed(completed, context.correlation_id())
        } else {
            ensure_upload_phase(&snapshot, MultipartJournalPhaseV1::Uploading)?;
            if command.scoped.now >= snapshot.expires_at() {
                return Err(not_found());
            }
            let list = self
                .provider
                .list_parts(context, provider_reference(context, &snapshot)?)
                .await?;
            validate_parts_list(context, &snapshot, &list, true)?;
            let request = ProviderCompleteMultipartRequestV1::new(
                provider_reference(context, &snapshot)?,
                list.parts().to_vec(),
                snapshot.spec().total_size(),
                snapshot.spec().checksum_sha256().clone(),
                snapshot.spec().content_type().clone(),
            )?;
            let completed = self.provider.complete_multipart(context, request).await?;
            validate_completed(context, &snapshot, &completed)?;
            completed
        };
        let expected = completed.clone();
        let returned = match self
            .journal
            .record_provider_complete(
                context,
                MultipartReplayKeyV1::client(command.idempotency_key),
                fingerprint,
                completed,
            )
            .await?
        {
            JournalMutationOutcomeV1::Applied(completed)
            | JournalMutationOutcomeV1::Replay(completed) => completed,
        };
        if returned != expected {
            return Err(integrity());
        }
        validate_completed(context, &snapshot, &returned)?;
        Ok(returned)
    }

    pub async fn finalize(
        &self,
        context: StorageRequestContext,
        command: IdempotentMultipartCommandV1,
    ) -> Result<SourceFinalizeRecordV1, StorageFailure> {
        self.authorize_scoped(context, &command.scoped, MultipartOperationV1::Finalize)
            .await?;
        let snapshot = self
            .load_snapshot(context, command.scoped.upload_id, &command.scoped.key)
            .await?;
        let completed = snapshot.completed().ok_or_else(conflict)?;
        let fingerprint = finalize_fingerprint(completed);
        let record = if let Some(record) = self
            .journal
            .get_finalize(context, command.scoped.upload_id)
            .await?
        {
            validate_finalize_record(&snapshot, &record)?;
            rebind_finalize(&record, context.correlation_id())
        } else {
            ensure_upload_phase(&snapshot, MultipartJournalPhaseV1::ProviderCompleted)?;
            let record = finalize_record(completed, command.scoped.now, context.correlation_id());
            validate_finalize_record(&snapshot, &record)?;
            record
        };
        let expected = record.clone();
        let returned = match self
            .journal
            .finalize(
                context,
                MultipartReplayKeyV1::client(command.idempotency_key),
                fingerprint,
                record,
            )
            .await?
        {
            JournalMutationOutcomeV1::Applied(record) => {
                if record != expected {
                    return Err(integrity());
                }
                record
            }
            JournalMutationOutcomeV1::Replay(record) => {
                if !same_finalize_result(&record, &expected) {
                    return Err(integrity());
                }
                record
            }
        };
        validate_finalize_record(&snapshot, &returned)?;
        let durable = self
            .journal
            .get_finalize(context, command.scoped.upload_id)
            .await?
            .ok_or_else(integrity)?;
        validate_finalize_record(&snapshot, &durable)?;
        if !same_durable_finalize(&returned, &durable) {
            return Err(integrity());
        }
        Ok(returned)
    }

    pub async fn abort(
        &self,
        context: StorageRequestContext,
        command: IdempotentMultipartCommandV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        self.authorize_scoped(context, &command.scoped, MultipartOperationV1::Abort)
            .await?;
        let snapshot = self
            .load_snapshot(context, command.scoped.upload_id, &command.scoped.key)
            .await?;
        let receipt = match snapshot.phase() {
            MultipartJournalPhaseV1::Aborted => ProviderAbortReceiptV1::new(
                snapshot.upload_id(),
                snapshot.spec().key().clone(),
                ProviderAbortDispositionV1::AlreadyAborted,
                context.correlation_id(),
            ),
            MultipartJournalPhaseV1::ProviderCompleted | MultipartJournalPhaseV1::Finalized => {
                ProviderAbortReceiptV1::new(
                    snapshot.upload_id(),
                    snapshot.spec().key().clone(),
                    ProviderAbortDispositionV1::AlreadyCompleted,
                    context.correlation_id(),
                )
            }
            MultipartJournalPhaseV1::Creating => return Err(conflict()),
            MultipartJournalPhaseV1::Uploading => {
                self.provider
                    .abort_multipart(context, provider_reference(context, &snapshot)?)
                    .await?
            }
        };
        validate_abort(context, &snapshot, &receipt)?;
        let fingerprint = abort_fingerprint(&snapshot);
        let expected = receipt.clone();
        let returned = match self
            .journal
            .abort(
                context,
                MultipartReplayKeyV1::client(command.idempotency_key),
                fingerprint,
                receipt,
            )
            .await?
        {
            JournalMutationOutcomeV1::Applied(receipt)
            | JournalMutationOutcomeV1::Replay(receipt) => receipt,
        };
        if !same_abort_result(&returned, &expected) {
            return Err(integrity());
        }
        validate_abort(context, &snapshot, &returned)?;
        Ok(returned)
    }

    pub async fn download(
        &self,
        context: StorageRequestContext,
        command: PrivateDownloadCommandV1,
    ) -> Result<PrivateDownloadResponseV1, StorageFailure> {
        if command.key.tenant_id() != context.tenant_id() {
            return Err(not_found());
        }
        let operation = match (command.method, command.range) {
            (PrivateDownloadMethodV1::Head, None) => MultipartOperationV1::Head,
            (PrivateDownloadMethodV1::Get, None) => MultipartOperationV1::Get,
            (PrivateDownloadMethodV1::Get, Some(_)) => MultipartOperationV1::Range,
            (PrivateDownloadMethodV1::Head, Some(_)) => return Err(not_found()),
        };
        self.authorize(
            context,
            &command.authorization,
            &command.key,
            None,
            operation,
            command.now,
        )
        .await?;
        if command
            .origin
            .as_ref()
            .is_some_and(|origin| !self.download_policy.allows(origin))
        {
            return Err(not_found());
        }
        let finalized = self.load_finalized_identity(context, &command.key).await?;
        let capabilities = self.provider.capabilities();
        capabilities.require(match command.method {
            PrivateDownloadMethodV1::Head => MultipartProviderOperationV1::Head,
            PrivateDownloadMethodV1::Get => MultipartProviderOperationV1::Get,
        })?;
        if let Some(range) = command.range {
            let size = range_size(range)?;
            if size > capabilities.max_range_size() {
                return Err(invalid());
            }
        }
        let request = ProviderDownloadRequestV1::new(
            command.key.clone(),
            command.range,
            command.validator.clone(),
            context.correlation_id(),
        );
        let provider_response = match command.method {
            PrivateDownloadMethodV1::Head => self.provider.head_private(context, request).await?,
            PrivateDownloadMethodV1::Get => self.provider.get_private(context, request).await?,
        };
        self.validate_download(
            context,
            &command,
            &finalized,
            capabilities.max_range_size(),
            provider_response,
        )
    }

    pub async fn reconcile(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
        now: TimestampMillis,
    ) -> Result<MultipartReconcileOutcomeV1, StorageFailure> {
        let snapshot = self
            .journal
            .get_upload(context, upload_id)
            .await?
            .ok_or_else(not_found)?;
        validate_snapshot_structure(context, &snapshot)?;
        if snapshot.upload_id() != upload_id {
            return Err(integrity());
        }
        match snapshot.phase() {
            MultipartJournalPhaseV1::Finalized | MultipartJournalPhaseV1::Aborted => Ok(
                MultipartReconcileOutcomeV1::AlreadyTerminal(snapshot.phase()),
            ),
            MultipartJournalPhaseV1::Creating => {
                self.preflight_spec(snapshot.spec())?;
                let recovered = self.lookup_provider_session(context, &snapshot).await?;
                if now >= snapshot.expires_at() {
                    let receipt = if let Some(session) = recovered {
                        let reference = ProviderUploadReferenceV1::new(
                            session.upload_id(),
                            session.key().clone(),
                            session.handle().clone(),
                            context.correlation_id(),
                        );
                        self.provider.abort_multipart(context, reference).await?
                    } else {
                        ProviderAbortReceiptV1::new(
                            snapshot.upload_id(),
                            snapshot.spec().key().clone(),
                            ProviderAbortDispositionV1::AlreadyAborted,
                            context.correlation_id(),
                        )
                    };
                    validate_abort(context, &snapshot, &receipt)?;
                    let receipt = self
                        .record_reconcile_abort(context, &snapshot, receipt)
                        .await?;
                    return Ok(MultipartReconcileOutcomeV1::Aborted(receipt));
                }
                let session = match recovered {
                    Some(session) => session,
                    None => self.create_provider_session(context, &snapshot).await?,
                };
                let activated = self
                    .journal
                    .activate_upload(context, session.clone())
                    .await?;
                validate_snapshot_structure(context, &activated)?;
                if activated.phase() != MultipartJournalPhaseV1::Uploading
                    || activated.upload_id() != snapshot.upload_id()
                    || activated
                        .provider_session()
                        .map(ProviderMultipartSessionV1::handle)
                        != Some(session.handle())
                {
                    return Err(integrity());
                }
                Ok(MultipartReconcileOutcomeV1::Activated(
                    MultipartUploadPlanV1::from_session(snapshot.spec(), &session),
                ))
            }
            MultipartJournalPhaseV1::Uploading if now >= snapshot.expires_at() => {
                let receipt = self
                    .provider
                    .abort_multipart(context, provider_reference(context, &snapshot)?)
                    .await?;
                validate_abort(context, &snapshot, &receipt)?;
                let receipt = self
                    .record_reconcile_abort(context, &snapshot, receipt)
                    .await?;
                Ok(MultipartReconcileOutcomeV1::Aborted(receipt))
            }
            MultipartJournalPhaseV1::Uploading => {
                let list = self
                    .provider
                    .list_parts(context, provider_reference(context, &snapshot)?)
                    .await?;
                validate_parts_list(context, &snapshot, &list, false)?;
                if list.parts().len() != usize::from(snapshot.spec().part_count()) {
                    return self
                        .resume_plan(context, &snapshot, list)
                        .map(MultipartReconcileOutcomeV1::WaitingForParts);
                }
                validate_parts_list(context, &snapshot, &list, true)?;
                let request = ProviderCompleteMultipartRequestV1::new(
                    provider_reference(context, &snapshot)?,
                    list.parts().to_vec(),
                    snapshot.spec().total_size(),
                    snapshot.spec().checksum_sha256().clone(),
                    snapshot.spec().content_type().clone(),
                )?;
                let completed = self.provider.complete_multipart(context, request).await?;
                validate_completed(context, &snapshot, &completed)?;
                let expected = completed.clone();
                let completed = match self
                    .journal
                    .record_provider_complete(
                        context,
                        MultipartReplayKeyV1::reconciliation(
                            MultipartSystemReplayOperationV1::Complete,
                            upload_id,
                        ),
                        complete_fingerprint(&snapshot),
                        completed,
                    )
                    .await?
                {
                    JournalMutationOutcomeV1::Applied(completed)
                    | JournalMutationOutcomeV1::Replay(completed) => completed,
                };
                if completed != expected {
                    return Err(integrity());
                }
                validate_completed(context, &snapshot, &completed)?;
                Ok(MultipartReconcileOutcomeV1::ProviderCompleted(completed))
            }
            MultipartJournalPhaseV1::ProviderCompleted => {
                if let Some(record) = self.journal.get_finalize(context, upload_id).await? {
                    validate_finalize_record(&snapshot, &record)?;
                    return Ok(MultipartReconcileOutcomeV1::Finalized(record));
                }
                let completed = snapshot.completed().ok_or_else(integrity)?;
                let record = finalize_record(completed, now, context.correlation_id());
                validate_finalize_record(&snapshot, &record)?;
                let expected = record.clone();
                let record = match self
                    .journal
                    .finalize(
                        context,
                        MultipartReplayKeyV1::reconciliation(
                            MultipartSystemReplayOperationV1::Finalize,
                            upload_id,
                        ),
                        finalize_fingerprint(completed),
                        record,
                    )
                    .await?
                {
                    JournalMutationOutcomeV1::Applied(record) => {
                        if record != expected {
                            return Err(integrity());
                        }
                        record
                    }
                    JournalMutationOutcomeV1::Replay(record) => {
                        if !same_finalize_result(&record, &expected) {
                            return Err(integrity());
                        }
                        record
                    }
                };
                validate_finalize_record(&snapshot, &record)?;
                let durable = self
                    .journal
                    .get_finalize(context, upload_id)
                    .await?
                    .ok_or_else(integrity)?;
                validate_finalize_record(&snapshot, &durable)?;
                if !same_durable_finalize(&record, &durable) {
                    return Err(integrity());
                }
                Ok(MultipartReconcileOutcomeV1::Finalized(record))
            }
        }
    }

    pub async fn reconciliation_candidates(
        &self,
        context: StorageRequestContext,
        limit: u16,
    ) -> Result<Vec<MultipartUploadSnapshotV1>, StorageFailure> {
        let candidates = self
            .journal
            .reconciliation_candidates(context, limit)
            .await?;
        for candidate in &candidates {
            validate_snapshot_structure(context, candidate)?;
        }
        Ok(candidates)
    }

    async fn record_reconcile_abort(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
        receipt: ProviderAbortReceiptV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        let expected = receipt.clone();
        let returned = match self
            .journal
            .abort(
                context,
                MultipartReplayKeyV1::reconciliation(
                    MultipartSystemReplayOperationV1::Abort,
                    snapshot.upload_id(),
                ),
                abort_fingerprint(snapshot),
                receipt,
            )
            .await?
        {
            JournalMutationOutcomeV1::Applied(receipt)
            | JournalMutationOutcomeV1::Replay(receipt) => receipt,
        };
        if !same_abort_result(&returned, &expected) {
            return Err(integrity());
        }
        validate_abort(context, snapshot, &returned)?;
        Ok(returned)
    }

    async fn authorize_scoped(
        &self,
        context: StorageRequestContext,
        command: &ScopedMultipartCommandV1,
        operation: MultipartOperationV1,
    ) -> Result<MultipartGrantRecordV1, StorageFailure> {
        self.authorize(
            context,
            &command.authorization,
            &command.key,
            Some(command.upload_id),
            operation,
            command.now,
        )
        .await
    }

    async fn authorize(
        &self,
        context: StorageRequestContext,
        authorization: &MultipartAuthorizationV1,
        key: &ScopedObjectKey,
        upload_id: Option<MultipartUploadId>,
        operation: MultipartOperationV1,
        now: TimestampMillis,
    ) -> Result<MultipartGrantRecordV1, StorageFailure> {
        if key.tenant_id() != context.tenant_id() {
            return Err(not_found());
        }
        let record = self
            .journal
            .get_grant(context, authorization.grant_id)
            .await?
            .ok_or_else(not_found)?;
        let scope = record.scope();
        let digest = self
            .keys
            .digest(record.key_version(), &authorization.grant_secret)
            .ok_or_else(not_found)?;
        if !record.active_at(now)
            || record.id() != authorization.grant_id
            || scope.tenant_id() != context.tenant_id()
            || scope.key() != key
            || scope.upload_id() != upload_id
            || scope.operation() != operation
            || !constant_time_eq(
                record.digest().expose_for_verification().as_bytes(),
                digest.expose_for_verification().as_bytes(),
            )
        {
            return Err(not_found());
        }
        Ok(record)
    }

    fn preflight_spec(&self, spec: &MultipartUploadSpecV1) -> Result<(), StorageFailure> {
        if spec.protocol_version() != frame_domain::MULTIPART_PROTOCOL_VERSION
            || spec.total_size() > self.limits.max_total_size()
            || spec.part_size() < self.limits.min_part_size()
            || spec.part_size() > self.limits.max_part_size()
            || spec.part_size() > self.limits.max_worker_request_size()
            || spec.part_count() > self.limits.max_part_count()
        {
            return Err(invalid());
        }
        let capabilities = self.provider.capabilities();
        for operation in [
            MultipartProviderOperationV1::Create,
            MultipartProviderOperationV1::Lookup,
            MultipartProviderOperationV1::ListParts,
            MultipartProviderOperationV1::PutPart,
            MultipartProviderOperationV1::Complete,
            MultipartProviderOperationV1::Abort,
        ] {
            capabilities.require(operation)?;
        }
        if !capabilities.checksum_sha256() {
            return Err(StorageFailure::unsupported());
        }
        if spec.part_size() < capabilities.min_part_size()
            || spec.part_size() > capabilities.max_part_size()
            || spec.part_count() > capabilities.max_part_count()
            || spec.total_size() > capabilities.max_total_size()
        {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        Ok(())
    }

    fn validate_session(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
        session: &ProviderMultipartSessionV1,
    ) -> Result<(), StorageFailure> {
        if session.upload_id() != snapshot.upload_id()
            || session.key() != snapshot.spec().key()
            || session.expires_at() != snapshot.expires_at()
            || session.correlation_id() != context.correlation_id()
            || session.handle().expose_for_provider().is_empty()
        {
            return Err(integrity());
        }
        Ok(())
    }

    async fn lookup_provider_session(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure> {
        let request = ProviderLookupMultipartRequestV1::new(
            snapshot.upload_id(),
            snapshot.spec().key().clone(),
            context.correlation_id(),
        );
        let session = self.provider.lookup_multipart(context, request).await?;
        if let Some(session) = &session {
            self.validate_session(context, snapshot, session)?;
        }
        Ok(session)
    }

    async fn create_provider_session(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        let request = ProviderCreateMultipartRequestV1::new(
            snapshot.upload_id(),
            snapshot.spec().clone(),
            snapshot.expires_at(),
            context.correlation_id(),
        );
        let session = self.provider.create_multipart(context, request).await?;
        self.validate_session(context, snapshot, &session)?;
        Ok(session)
    }

    async fn create_or_recover_session(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        match self.lookup_provider_session(context, snapshot).await? {
            Some(session) => Ok(session),
            None => self.create_provider_session(context, snapshot).await,
        }
    }

    fn plan_from_snapshot(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
    ) -> Result<MultipartUploadPlanV1, StorageFailure> {
        validate_snapshot_structure(context, snapshot)?;
        let session = snapshot.provider_session().ok_or_else(integrity)?;
        let rebound = ProviderMultipartSessionV1::new(
            session.upload_id(),
            session.key().clone(),
            session.handle().clone(),
            session.expires_at(),
            context.correlation_id(),
        );
        Ok(MultipartUploadPlanV1::from_session(
            snapshot.spec(),
            &rebound,
        ))
    }

    async fn load_active(
        &self,
        context: StorageRequestContext,
        command: &ScopedMultipartCommandV1,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure> {
        let snapshot = self
            .load_snapshot(context, command.upload_id, &command.key)
            .await?;
        ensure_upload_phase(&snapshot, MultipartJournalPhaseV1::Uploading)?;
        if command.now >= snapshot.expires_at() {
            return Err(not_found());
        }
        Ok(snapshot)
    }

    async fn load_snapshot(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
        key: &ScopedObjectKey,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure> {
        let snapshot = self
            .journal
            .get_upload(context, upload_id)
            .await?
            .ok_or_else(not_found)?;
        if snapshot.upload_id() != upload_id || snapshot.spec().key() != key {
            return Err(not_found());
        }
        validate_snapshot_structure(context, &snapshot)?;
        Ok(snapshot)
    }

    async fn load_finalized_identity(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<SourceFinalizeRecordV1, StorageFailure> {
        let record = self
            .journal
            .get_finalize_by_key(context, key.clone())
            .await?
            .ok_or_else(not_found)?;
        if record.key() != key {
            return Err(integrity());
        }
        let snapshot = self.load_snapshot(context, record.upload_id(), key).await?;
        ensure_upload_phase(&snapshot, MultipartJournalPhaseV1::Finalized)?;
        validate_finalize_record(&snapshot, &record)?;
        let by_upload = self
            .journal
            .get_finalize(context, record.upload_id())
            .await?
            .ok_or_else(integrity)?;
        if by_upload != record {
            return Err(integrity());
        }
        Ok(record)
    }

    fn resume_plan(
        &self,
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
        list: ProviderPartsListV1,
    ) -> Result<MultipartResumePlanV1, StorageFailure> {
        validate_parts_list(context, snapshot, &list, false)?;
        Ok(MultipartResumePlanV1 {
            upload_id: snapshot.upload_id(),
            key: snapshot.spec().key().clone(),
            verified_parts: list.parts().to_vec(),
            retry_directives: [
                MultipartRetryDirectiveV1::PersistJournalBeforeTransfer,
                MultipartRetryDirectiveV1::RetrySamePartAndIdempotencyKey,
                MultipartRetryDirectiveV1::ListVerifiedPartsAfterRestart,
                MultipartRetryDirectiveV1::RestartAfterExpiry,
            ],
            expires_at: snapshot.expires_at(),
            correlation_id: context.correlation_id(),
        })
    }

    fn validate_download(
        &self,
        context: StorageRequestContext,
        command: &PrivateDownloadCommandV1,
        finalized: &SourceFinalizeRecordV1,
        maximum_chunk_size: ByteSize,
        response: ProviderDownloadResponseV1,
    ) -> Result<PrivateDownloadResponseV1, StorageFailure> {
        let metadata = response.metadata().clone();
        validate_download_metadata(context, &command.key, finalized, &metadata)?;
        validate_validator(&command.validator, &response)?;
        let (status, content_length, content_range, body) = match response {
            ProviderDownloadResponseV1::NotModified(_) => {
                (PrivateDownloadStatusV1::NotModified, None, None, None)
            }
            ProviderDownloadResponseV1::Head(_)
                if command.method == PrivateDownloadMethodV1::Head =>
            {
                (
                    PrivateDownloadStatusV1::Ok,
                    Some(metadata.size()),
                    None,
                    None,
                )
            }
            ProviderDownloadResponseV1::Body { range, body, .. }
                if command.method == PrivateDownloadMethodV1::Get =>
            {
                let expected_range = command
                    .range
                    .unwrap_or(ObjectByteRange::new(0, metadata.size().get())?);
                if range != expected_range || range.end_exclusive() > metadata.size().get() {
                    return Err(integrity());
                }
                let status = if command.range.is_some() {
                    PrivateDownloadStatusV1::PartialContent
                } else {
                    PrivateDownloadStatusV1::Ok
                };
                (
                    status,
                    Some(range_size(range)?),
                    command
                        .range
                        .map(|_| PrivateDownloadContentRangeV1::new(range, metadata.size())),
                    Some(PrivateDownloadBodyV1::new(
                        body,
                        range_size(range)?,
                        maximum_chunk_size,
                        command
                            .range
                            .is_none()
                            .then(|| finalized.checksum_sha256().clone()),
                    )),
                )
            }
            ProviderDownloadResponseV1::Head(_) | ProviderDownloadResponseV1::Body { .. } => {
                return Err(integrity());
            }
        };
        Ok(PrivateDownloadResponseV1 {
            status,
            headers: PrivateDownloadHeadersV1 {
                content_length,
                content_range,
                content_type: metadata.content_type().clone(),
                disposition: command.disposition,
                etag: metadata.provider_etag().clone(),
                last_modified: metadata.last_modified(),
                cache_policy: self.download_policy.cache_policy(),
                cors_allow_origin: command.origin.clone(),
                vary_origin: command.origin.is_some(),
                accept_ranges: true,
            },
            body,
            correlation_id: context.correlation_id(),
        })
    }
}

fn validate_snapshot_structure(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
) -> Result<(), StorageFailure> {
    if snapshot.spec().protocol_version() != frame_domain::MULTIPART_PROTOCOL_VERSION
        || snapshot.spec().key().tenant_id() != context.tenant_id()
    {
        return Err(integrity());
    }
    if let Some(session) = snapshot.provider_session()
        && (session.upload_id() != snapshot.upload_id()
            || session.key() != snapshot.spec().key()
            || session.expires_at() != snapshot.expires_at()
            || session.handle().expose_for_provider().is_empty())
    {
        return Err(integrity());
    }
    let mut previous_part = 0;
    for part in snapshot.parts() {
        if part.upload_id() != snapshot.upload_id()
            || part.key() != snapshot.spec().key()
            || part.part_number().get() <= previous_part
            || snapshot
                .spec()
                .validate_part(part.part_number(), part.size())
                .is_err()
            || part.etag().expose_for_provider_comparison().is_empty()
        {
            return Err(integrity());
        }
        previous_part = part.part_number().get();
    }
    if let Some(completed) = snapshot.completed() {
        validate_completed_identity(snapshot, completed)?;
    }
    let shape_is_valid = match snapshot.phase() {
        MultipartJournalPhaseV1::Creating => {
            snapshot.provider_session().is_none()
                && snapshot.parts().is_empty()
                && snapshot.completed().is_none()
        }
        MultipartJournalPhaseV1::Uploading => {
            snapshot.provider_session().is_some() && snapshot.completed().is_none()
        }
        MultipartJournalPhaseV1::ProviderCompleted | MultipartJournalPhaseV1::Finalized => {
            snapshot.provider_session().is_some() && snapshot.completed().is_some()
        }
        MultipartJournalPhaseV1::Aborted => snapshot.completed().is_none(),
    };
    if !shape_is_valid {
        return Err(integrity());
    }
    Ok(())
}

fn provider_reference(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
) -> Result<ProviderUploadReferenceV1, StorageFailure> {
    let session = snapshot.provider_session().ok_or_else(conflict)?;
    Ok(ProviderUploadReferenceV1::new(
        snapshot.upload_id(),
        snapshot.spec().key().clone(),
        session.handle().clone(),
        context.correlation_id(),
    ))
}

fn ensure_upload_phase(
    snapshot: &MultipartUploadSnapshotV1,
    expected: MultipartJournalPhaseV1,
) -> Result<(), StorageFailure> {
    if snapshot.phase() == expected {
        Ok(())
    } else {
        Err(conflict())
    }
}

fn validate_part_receipt(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
    part_number: MultipartPartNumberV1,
    size: ByteSize,
    checksum: &ChecksumSha256,
    receipt: &ProviderPartReceiptV1,
) -> Result<(), StorageFailure> {
    if receipt.upload_id() != snapshot.upload_id()
        || receipt.key() != snapshot.spec().key()
        || receipt.part_number() != part_number
        || receipt.size() != size
        || receipt.checksum_sha256() != checksum
        || receipt.correlation_id() != context.correlation_id()
        || receipt.etag().expose_for_provider_comparison().is_empty()
    {
        return Err(integrity());
    }
    Ok(())
}

fn validate_parts_list(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
    list: &ProviderPartsListV1,
    require_complete: bool,
) -> Result<(), StorageFailure> {
    if list.upload_id() != snapshot.upload_id()
        || list.key() != snapshot.spec().key()
        || list.correlation_id() != context.correlation_id()
        || list.parts().len() > usize::from(snapshot.spec().part_count())
        || (require_complete && list.parts().len() != usize::from(snapshot.spec().part_count()))
    {
        return Err(integrity());
    }
    let mut previous = 0;
    for (index, part) in list.parts().iter().enumerate() {
        let expected_number = if require_complete {
            Some(
                MultipartPartNumberV1::new(u16::try_from(index + 1).map_err(|_| integrity())?)
                    .map_err(|_| integrity())?,
            )
        } else {
            None
        };
        if part.upload_id() != snapshot.upload_id()
            || part.key() != snapshot.spec().key()
            || expected_number.is_some_and(|expected| part.part_number() != expected)
            || part.part_number().get() <= previous
            || part.part_number().get() > snapshot.spec().part_count()
            || snapshot
                .spec()
                .validate_part(part.part_number(), part.size())
                .is_err()
            || part.correlation_id() != context.correlation_id()
        {
            return Err(integrity());
        }
        previous = part.part_number().get();
    }
    Ok(())
}

fn validate_completed(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
    completed: &ProviderCompletedObjectV1,
) -> Result<(), StorageFailure> {
    validate_completed_identity(snapshot, completed)?;
    if completed.correlation_id() != context.correlation_id() {
        return Err(integrity());
    }
    Ok(())
}

fn validate_completed_identity(
    snapshot: &MultipartUploadSnapshotV1,
    completed: &ProviderCompletedObjectV1,
) -> Result<(), StorageFailure> {
    if completed.upload_id() != snapshot.upload_id()
        || completed.key() != snapshot.spec().key()
        || completed.size() != snapshot.spec().total_size()
        || completed.checksum_sha256() != snapshot.spec().checksum_sha256()
        || completed.content_type() != snapshot.spec().content_type()
        || completed
            .provider_version()
            .expose_for_provider_comparison()
            .is_empty()
        || completed
            .provider_etag()
            .expose_for_provider_comparison()
            .is_empty()
    {
        return Err(integrity());
    }
    Ok(())
}

fn validate_finalize_record(
    snapshot: &MultipartUploadSnapshotV1,
    record: &SourceFinalizeRecordV1,
) -> Result<(), StorageFailure> {
    let completed = snapshot.completed().ok_or_else(integrity)?;
    if record.upload_id() != snapshot.upload_id()
        || record.key() != snapshot.spec().key()
        || record.provider_version() != completed.provider_version()
        || record.provider_etag() != completed.provider_etag()
        || record.size() != completed.size()
        || record.checksum_sha256() != completed.checksum_sha256()
        || record.content_type() != completed.content_type()
        || record.provider_last_modified() != completed.last_modified()
        || record.media_probe() != completed.media_probe()
        || record.finalized_at() < completed.last_modified()
    {
        return Err(integrity());
    }
    Ok(())
}

fn same_finalize_result(left: &SourceFinalizeRecordV1, right: &SourceFinalizeRecordV1) -> bool {
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.provider_version() == right.provider_version()
        && left.provider_etag() == right.provider_etag()
        && left.size() == right.size()
        && left.checksum_sha256() == right.checksum_sha256()
        && left.content_type() == right.content_type()
        && left.provider_last_modified() == right.provider_last_modified()
        && left.media_probe() == right.media_probe()
        && left.correlation_id() == right.correlation_id()
}

fn same_durable_finalize(left: &SourceFinalizeRecordV1, right: &SourceFinalizeRecordV1) -> bool {
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.provider_version() == right.provider_version()
        && left.provider_etag() == right.provider_etag()
        && left.size() == right.size()
        && left.checksum_sha256() == right.checksum_sha256()
        && left.content_type() == right.content_type()
        && left.provider_last_modified() == right.provider_last_modified()
        && left.media_probe() == right.media_probe()
        && left.finalized_at() == right.finalized_at()
}

fn validate_abort(
    context: StorageRequestContext,
    snapshot: &MultipartUploadSnapshotV1,
    receipt: &ProviderAbortReceiptV1,
) -> Result<(), StorageFailure> {
    if receipt.upload_id() != snapshot.upload_id()
        || receipt.key() != snapshot.spec().key()
        || receipt.correlation_id() != context.correlation_id()
    {
        return Err(integrity());
    }
    Ok(())
}

fn same_abort_result(left: &ProviderAbortReceiptV1, right: &ProviderAbortReceiptV1) -> bool {
    let compatible_disposition = left.disposition() == right.disposition()
        || matches!(
            (left.disposition(), right.disposition()),
            (
                ProviderAbortDispositionV1::Aborted,
                ProviderAbortDispositionV1::AlreadyAborted
            ) | (
                ProviderAbortDispositionV1::AlreadyAborted,
                ProviderAbortDispositionV1::Aborted
            )
        );
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.correlation_id() == right.correlation_id()
        && compatible_disposition
}

fn validate_download_metadata(
    context: StorageRequestContext,
    key: &ScopedObjectKey,
    finalized: &SourceFinalizeRecordV1,
    metadata: &ProviderDownloadMetadataV1,
) -> Result<(), StorageFailure> {
    if metadata.key() != key
        || metadata.key().tenant_id() != context.tenant_id()
        || finalized.key() != key
        || metadata.size() != finalized.size()
        || metadata.checksum_sha256() != finalized.checksum_sha256()
        || metadata.content_type() != finalized.content_type()
        || metadata.provider_version() != finalized.provider_version()
        || metadata.provider_etag() != finalized.provider_etag()
        || metadata.last_modified() != finalized.provider_last_modified()
        || metadata.correlation_id() != context.correlation_id()
        || metadata
            .provider_version()
            .expose_for_provider_comparison()
            .is_empty()
        || metadata
            .provider_etag()
            .expose_for_provider_comparison()
            .is_empty()
    {
        return Err(integrity());
    }
    Ok(())
}

fn validate_validator(
    validator: &DownloadValidatorV1,
    response: &ProviderDownloadResponseV1,
) -> Result<(), StorageFailure> {
    let etag = response.metadata().provider_etag();
    let valid = match (validator, response) {
        (DownloadValidatorV1::None, ProviderDownloadResponseV1::NotModified(_)) => false,
        (DownloadValidatorV1::None, _) => true,
        (DownloadValidatorV1::IfMatch(expected), ProviderDownloadResponseV1::NotModified(_)) => {
            let _ = expected;
            false
        }
        (DownloadValidatorV1::IfMatch(expected), _) => expected == etag,
        (
            DownloadValidatorV1::IfNoneMatch(expected),
            ProviderDownloadResponseV1::NotModified(_),
        ) => expected == etag,
        (DownloadValidatorV1::IfNoneMatch(expected), _) => expected != etag,
    };
    if valid { Ok(()) } else { Err(integrity()) }
}

fn finalize_record(
    completed: &ProviderCompletedObjectV1,
    finalized_at: TimestampMillis,
    correlation_id: frame_domain::CorrelationId,
) -> SourceFinalizeRecordV1 {
    SourceFinalizeRecordV1::new(
        completed.upload_id(),
        completed.key().clone(),
        completed.provider_version().clone(),
        completed.provider_etag().clone(),
        completed.size(),
        completed.checksum_sha256().clone(),
        completed.content_type().clone(),
        completed.last_modified(),
        completed.media_probe().clone(),
        finalized_at,
        correlation_id,
    )
}

fn rebind_completed(
    completed: &ProviderCompletedObjectV1,
    correlation_id: frame_domain::CorrelationId,
) -> ProviderCompletedObjectV1 {
    ProviderCompletedObjectV1::new(
        completed.upload_id(),
        completed.key().clone(),
        completed.size(),
        completed.checksum_sha256().clone(),
        completed.content_type().clone(),
        completed.provider_version().clone(),
        completed.provider_etag().clone(),
        completed.last_modified(),
        completed.media_probe().clone(),
        correlation_id,
    )
}

fn rebind_finalize(
    record: &SourceFinalizeRecordV1,
    correlation_id: frame_domain::CorrelationId,
) -> SourceFinalizeRecordV1 {
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
        record.finalized_at(),
        correlation_id,
    )
}

fn create_fingerprint(spec: &MultipartUploadSpecV1, expires_at: TimestampMillis) -> ChecksumSha256 {
    fingerprint(
        b"create",
        &[
            spec.key().as_str().as_bytes(),
            &spec.total_size().get().to_be_bytes(),
            &spec.part_size().get().to_be_bytes(),
            spec.checksum_sha256().as_str().as_bytes(),
            spec.content_type().as_str().as_bytes(),
            &expires_at.get().to_be_bytes(),
        ],
    )
}

fn part_fingerprint(
    upload_id: MultipartUploadId,
    key: &ScopedObjectKey,
    part_number: MultipartPartNumberV1,
    size: ByteSize,
    checksum: &ChecksumSha256,
) -> ChecksumSha256 {
    fingerprint(
        b"part",
        &[
            upload_id.to_string().as_bytes(),
            key.as_str().as_bytes(),
            &part_number.get().to_be_bytes(),
            &size.get().to_be_bytes(),
            checksum.as_str().as_bytes(),
        ],
    )
}

fn complete_fingerprint(snapshot: &MultipartUploadSnapshotV1) -> ChecksumSha256 {
    fingerprint(
        b"complete",
        &[
            snapshot.upload_id().to_string().as_bytes(),
            snapshot.spec().key().as_str().as_bytes(),
            snapshot.spec().checksum_sha256().as_str().as_bytes(),
            &snapshot.spec().total_size().get().to_be_bytes(),
        ],
    )
}

fn finalize_fingerprint(completed: &ProviderCompletedObjectV1) -> ChecksumSha256 {
    fingerprint(
        b"finalize",
        &[
            completed.upload_id().to_string().as_bytes(),
            completed.key().as_str().as_bytes(),
            completed
                .provider_version()
                .expose_for_provider_comparison()
                .as_bytes(),
            completed.checksum_sha256().as_str().as_bytes(),
        ],
    )
}

fn abort_fingerprint(snapshot: &MultipartUploadSnapshotV1) -> ChecksumSha256 {
    fingerprint(
        b"abort",
        &[
            snapshot.upload_id().to_string().as_bytes(),
            snapshot.spec().key().as_str().as_bytes(),
        ],
    )
}

fn fingerprint(label: &[u8], values: &[&[u8]]) -> ChecksumSha256 {
    let mut framed = Vec::new();
    append_frame(&mut framed, b"frame.multipart.command.v1");
    append_frame(&mut framed, label);
    for value in values {
        append_frame(&mut framed, value);
    }
    ChecksumSha256::digest_bytes(&framed)
}

fn multipart_grant_digest(
    key: &[u8],
    domain: &[u8],
    version: MultipartGrantKeyVersion,
    secret: &[u8],
) -> Option<SecretDigest> {
    let mut message = Vec::new();
    append_frame(&mut message, domain);
    append_frame(&mut message, &version.get().to_be_bytes());
    append_frame(&mut message, secret);
    SecretDigest::parse_sha256(hex_lower(&hmac_sha256(key, &message))).ok()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> sha2::digest::Output<Sha256> {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for (index, value) in key_block.iter().enumerate() {
        inner_pad[index] ^= value;
        outer_pad[index] ^= value;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize()
}

fn append_frame(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    output.extend_from_slice(value);
}

fn hex_lower(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let maximum = left.len().max(right.len());
    for index in 0..maximum {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

fn range_size(range: ObjectByteRange) -> Result<ByteSize, StorageFailure> {
    ByteSize::new(
        range
            .end_exclusive()
            .checked_sub(range.start())
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())
}

fn invalid() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::InvalidRequest)
}

fn not_found() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::NotFound)
}

fn conflict() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::PreconditionFailed)
}

fn integrity() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Integrity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc_4231_case_one() {
        assert_eq!(
            hex_lower(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn multipart_grant_hmac_has_a_stable_known_answer_and_separates_domains_and_versions() {
        let version_one = MultipartGrantKeyVersion::new(1).expect("version");
        let version_two = MultipartGrantKeyVersion::new(2).expect("version");
        let key = [0x5a; 32];
        let secret = b"multipart-test-secret-material-0001";
        let expected =
            multipart_grant_digest(&key, b"frame.multipart.grant.v1", version_two, secret)
                .expect("digest");
        assert_eq!(
            expected.expose_for_verification(),
            "5414c0fa6537cf376f6ecc8401e88aa34094fc2ebb5753f10de7de819e1c8478"
        );
        assert_ne!(
            expected,
            multipart_grant_digest(&key, b"frame.multipart.other.v1", version_two, secret,)
                .expect("domain digest")
        );
        assert_ne!(
            expected,
            multipart_grant_digest(&key, b"frame.multipart.grant.v1", version_one, secret,)
                .expect("version digest")
        );
        assert_ne!(
            expected,
            multipart_grant_digest(
                &key,
                b"frame.multipart.grant.v1",
                version_two,
                b"multipart-test-secret-material-0002",
            )
            .expect("secret digest")
        );
    }

    #[test]
    fn grant_key_ring_rejects_duplicate_versions() {
        let version = MultipartGrantKeyVersion::new(1).expect("version");
        let first = MultipartGrantKeyMaterialV1::parse(vec![1; 32]).expect("key");
        let second = MultipartGrantKeyMaterialV1::parse(vec![2; 32]).expect("key");
        assert_eq!(
            MultipartGrantKeyRingV1::new(version, [(version, first), (version, second)])
                .expect_err("duplicate version")
                .kind(),
            StorageFailureKind::InvalidRequest
        );
    }
}
