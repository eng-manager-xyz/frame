use std::collections::{BTreeMap, HashMap, HashSet};

use frame_domain::{
    BackfillContractErrorV1, BackfillDiscrepancyKindV1, BackfillDiscrepancyV1, BackfillEntryIdV1,
    BackfillEntryStatusV1, BackfillExecutionPolicyV1, BackfillFailureClassV1,
    BackfillInventorySideV1, BackfillInventoryTotalsV1, BackfillLeaseV1, BackfillManifestIdV1,
    BackfillObjectFingerprintV1, BackfillOperationIdV1, BackfillOperationReceiptV1,
    BackfillOwnerDispositionV1, BackfillReconciliationDispositionV1, BackfillRunStateV1,
    BackfillWorkerIdV1, BackfillWriteResultV1, ByteSize, ChecksumSha256, DurationMillis,
    ObjectBackfillJournalV1, ObjectBackfillManifestEntryV1, ObjectBackfillManifestV1,
    ObjectBackfillReconciliationReportV1, ObjectRole, TenantId, TimestampMillis,
};
use frame_ports::{
    BackfillCancellationPortV1, BackfillChunkV1, BackfillClockPortV1, BackfillCommitFenceV1,
    BackfillConditionalCreateV1, BackfillCreateSpecV1, BackfillDestinationPortV1,
    BackfillInventoryCursorV1, BackfillJournalPortV1, BackfillManifestPortV1,
    BackfillMediaProbePortV1, BackfillObjectLocationV1, BackfillObjectMetadataV1,
    BackfillOpenedReadV1, BackfillOwnerApprovalCapabilityV1, BackfillOwnerApprovalPortV1,
    BackfillPortErrorV1, BackfillProbeReceiptV1, BackfillProbeSessionV1, BackfillProviderAccessV1,
    BackfillReadBodyV1, BackfillRuntimeBindingsV1, BackfillSourcePortV1,
    BackfillThrottleDecisionV1, BackfillThrottlePortV1, BackfillWriteBodyV1,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_CAS_ATTEMPTS: usize = 32;
const INVENTORY_PAGE_SIZE: u16 = 500;
const MAX_INVENTORY_PAGES: usize = 100_000;
const MAX_INVENTORY_OBJECTS: usize = 1_000_000;
const RECONCILIATION_MAX_AGE_MILLIS: u64 = 300_000;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ObjectBackfillErrorV1 {
    #[error("object-backfill manifest was not found")]
    ManifestNotFound,
    #[error("object-backfill journal was not found")]
    JournalNotFound,
    #[error("object-backfill runtime authority does not match the manifest")]
    AuthorityMismatch,
    #[error("object-backfill provider capability preflight failed")]
    CapabilityMissing,
    #[error("object-backfill state changed concurrently")]
    StateConflict,
    #[error("object-backfill worker lease was lost")]
    LeaseLost,
    #[error("object-backfill service is temporarily unavailable")]
    Unavailable,
    #[error("object-backfill contract is invalid")]
    InvalidContract,
    #[error("object-backfill clock moved backwards")]
    ClockRollback,
    #[error("object-backfill owner approval was denied")]
    ApprovalDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillProcessOutcomeV1 {
    Copied(BackfillEntryIdV1),
    ReusedExact(BackfillEntryIdV1),
    RetryScheduled {
        entry_id: BackfillEntryIdV1,
        failure: BackfillFailureClassV1,
    },
    Quarantined {
        entry_id: BackfillEntryIdV1,
        failure: BackfillFailureClassV1,
    },
    DeferredUntil(TimestampMillis),
    CircuitOpenUntil(TimestampMillis),
    BudgetExhausted,
    Idle,
    Paused,
    Aborted,
    Completed,
    LeaseLost(BackfillEntryIdV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransferSuccessV1 {
    result: BackfillWriteResultV1,
    destination_version_present: bool,
    probe_profile_version: Option<u16>,
}

#[derive(Debug)]
struct VerifiedObjectV1 {
    metadata: BackfillObjectMetadataV1,
    probe: Option<BackfillProbeReceiptV1>,
}

#[derive(Default, PartialEq, Eq)]
struct InventoryIndexV1 {
    by_location: BTreeMap<String, Vec<BackfillObjectMetadataV1>>,
    tenant_snapshots: BTreeMap<String, ChecksumSha256>,
    object_count: usize,
}

impl InventoryIndexV1 {
    fn insert(&mut self, metadata: BackfillObjectMetadataV1) -> Result<(), BackfillPortErrorV1> {
        self.object_count = self
            .object_count
            .checked_add(1)
            .filter(|count| *count <= MAX_INVENTORY_OBJECTS)
            .ok_or(BackfillPortErrorV1::InvalidResponse)?;
        self.by_location
            .entry(metadata_sort_key(&metadata))
            .or_default()
            .push(metadata);
        Ok(())
    }

    fn record_snapshot(
        &mut self,
        tenant: TenantId,
        digest: ChecksumSha256,
    ) -> Result<(), BackfillPortErrorV1> {
        match self
            .tenant_snapshots
            .insert(tenant.to_string(), digest.clone())
        {
            Some(previous) if previous != digest => Err(BackfillPortErrorV1::InvalidResponse),
            _ => Ok(()),
        }
    }

    fn source(
        &self,
        reference: &frame_domain::BackfillSourceReferenceV1,
    ) -> &[BackfillObjectMetadataV1] {
        self.by_location
            .get(&format!("source:{}", reference.as_str()))
            .map_or(&[], Vec::as_slice)
    }

    fn target(&self, key: &frame_domain::ScopedObjectKey) -> &[BackfillObjectMetadataV1] {
        self.by_location
            .get(&format!("target:{}", key.as_str()))
            .map_or(&[], Vec::as_slice)
    }

    fn all(&self) -> impl Iterator<Item = &BackfillObjectMetadataV1> {
        self.by_location.values().flatten()
    }
}

pub struct ObjectBackfillCoordinatorV1<'a> {
    manifests: &'a dyn BackfillManifestPortV1,
    journals: &'a dyn BackfillJournalPortV1,
    source: &'a dyn BackfillSourcePortV1,
    target: &'a dyn BackfillDestinationPortV1,
    probes: &'a dyn BackfillMediaProbePortV1,
    throttle: &'a dyn BackfillThrottlePortV1,
    cancellation: &'a dyn BackfillCancellationPortV1,
    clock: &'a dyn BackfillClockPortV1,
    approvals: &'a dyn BackfillOwnerApprovalPortV1,
}

impl<'a> ObjectBackfillCoordinatorV1<'a> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        manifests: &'a dyn BackfillManifestPortV1,
        journals: &'a dyn BackfillJournalPortV1,
        source: &'a dyn BackfillSourcePortV1,
        target: &'a dyn BackfillDestinationPortV1,
        probes: &'a dyn BackfillMediaProbePortV1,
        throttle: &'a dyn BackfillThrottlePortV1,
        cancellation: &'a dyn BackfillCancellationPortV1,
        clock: &'a dyn BackfillClockPortV1,
        approvals: &'a dyn BackfillOwnerApprovalPortV1,
    ) -> Self {
        Self {
            manifests,
            journals,
            source,
            target,
            probes,
            throttle,
            cancellation,
            clock,
            approvals,
        }
    }

    /// Persists the immutable manifest, capability-preflights both providers, and creates the
    /// mutable journal. Replays are exact and never replace either durable record.
    pub async fn initialize(
        &self,
        manifest: &ObjectBackfillManifestV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        manifest.validate_integrity().map_err(map_contract_error)?;
        let initialize_now = self.now_at_or_after(manifest.created_at())?;
        self.validate_bindings(manifest, bindings)?;
        self.preflight(bindings).await?;

        match self.manifests.put_immutable(manifest).await {
            Ok(()) => {}
            Err(BackfillPortErrorV1::Conflict | BackfillPortErrorV1::ProviderOutage) => {
                let stored = self
                    .manifests
                    .load(manifest.manifest_id())
                    .await
                    .map_err(map_durable_port_error)?
                    .ok_or(ObjectBackfillErrorV1::ManifestNotFound)?;
                if stored != *manifest {
                    return Err(ObjectBackfillErrorV1::StateConflict);
                }
            }
            Err(error) => return Err(map_durable_port_error(error)),
        }

        let journal = ObjectBackfillJournalV1::new(manifest);
        match self.journals.create(&journal).await {
            Ok(()) => Ok(journal),
            Err(BackfillPortErrorV1::Conflict | BackfillPortErrorV1::ProviderOutage) => {
                let stored = self
                    .journals
                    .load(manifest.manifest_id())
                    .await
                    .map_err(map_durable_port_error)?
                    .ok_or(ObjectBackfillErrorV1::JournalNotFound)?;
                stored
                    .validate_for_manifest_at(manifest, initialize_now)
                    .map_err(map_contract_error)?;
                Ok(stored)
            }
            Err(error) => Err(map_durable_port_error(error)),
        }
    }

    /// Claims and executes at most one deterministic entry for the exact tenant scope.
    pub async fn process_next(
        &self,
        manifest_id: BackfillManifestIdV1,
        tenant_scope: TenantId,
        bindings: &BackfillRuntimeBindingsV1<'_>,
        worker_id: BackfillWorkerIdV1,
    ) -> Result<BackfillProcessOutcomeV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        let policy = manifest.execution_policy();
        self.validate_bindings(&manifest, bindings)?;
        self.preflight(bindings).await?;

        let mut claimed = None;
        let mut foreign_expired_repaired = false;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let selection_now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(&manifest, selection_now).await?;
            match journal.state() {
                BackfillRunStateV1::Paused => return Ok(BackfillProcessOutcomeV1::Paused),
                BackfillRunStateV1::Aborting | BackfillRunStateV1::Aborted => {
                    return Ok(BackfillProcessOutcomeV1::Aborted);
                }
                BackfillRunStateV1::Completed => {
                    return Ok(BackfillProcessOutcomeV1::Completed);
                }
                BackfillRunStateV1::Running => {}
            }

            let half_open_token = journal.circuit().half_open_fencing_token();
            if let Some((entry_id, operation_id, _priority)) = journal
                .entries()
                .iter()
                .filter_map(|progress| {
                    let entry = manifest.entry(progress.entry_id())?;
                    match progress.status() {
                        BackfillEntryStatusV1::Leased {
                            lease,
                            operation_id,
                        } if lease.expired_at(selection_now) => {
                            let half_open = half_open_token == Some(lease.fencing_token());
                            let foreign = entry.tenant_id() != tenant_scope;
                            if foreign && foreign_expired_repaired && !half_open {
                                return None;
                            }
                            Some((
                                progress.entry_id(),
                                *operation_id,
                                match (half_open, foreign) {
                                    (true, _) => 0,
                                    (false, false) => 1,
                                    (false, true) => 2,
                                },
                            ))
                        }
                        _ => None,
                    }
                })
                .min_by_key(|(_, _, priority)| *priority)
            {
                let entry = manifest
                    .entry(entry_id)
                    .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
                let outcome = self
                    .recover_or_normalize_expired(
                        &manifest,
                        entry,
                        operation_id,
                        bindings,
                        selection_now,
                    )
                    .await?;
                if entry.tenant_id() == tenant_scope {
                    return Ok(outcome);
                }
                foreign_expired_repaired = true;
                continue;
            }
            if let Some(until) = journal.circuit().open_until()
                && until > selection_now
            {
                return Ok(BackfillProcessOutcomeV1::CircuitOpenUntil(until));
            }
            if journal.usage().admitted_entries() >= policy.max_entries_per_run() {
                return Ok(BackfillProcessOutcomeV1::BudgetExhausted);
            }
            let active_leases = journal
                .entries()
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.status(),
                        BackfillEntryStatusV1::Leased { lease, .. }
                            if !lease.expired_at(selection_now)
                    )
                })
                .count();
            if active_leases >= usize::from(policy.max_concurrency()) {
                return Ok(BackfillProcessOutcomeV1::Idle);
            }
            let entry_id = journal
                .entries()
                .iter()
                .find(|progress| {
                    progress.eligible_at(selection_now)
                        && !matches!(progress.status(), BackfillEntryStatusV1::Leased { .. })
                        && manifest
                            .entry(progress.entry_id())
                            .is_some_and(|entry| entry.tenant_id() == tenant_scope)
                })
                .map(frame_domain::BackfillJournalEntryV1::entry_id);
            let Some(entry_id) = entry_id else {
                return Ok(BackfillProcessOutcomeV1::Idle);
            };
            let entry = manifest
                .entry(entry_id)
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?;

            let admission_now = self.now_at_or_after(journal.last_transition_at())?;
            match self
                .throttle
                .admit_object(
                    entry.tenant_id(),
                    manifest.source().provider(),
                    manifest.source().region(),
                    manifest.target().provider(),
                    manifest.target().region(),
                    policy.max_objects_per_minute(),
                    admission_now,
                )
                .await
                .map_err(map_durable_port_error)?
            {
                BackfillThrottleDecisionV1::Allowed => {}
                BackfillThrottleDecisionV1::DeferredUntil(until) => {
                    return Ok(BackfillProcessOutcomeV1::DeferredUntil(until));
                }
            }

            let source_cost_units = self
                .source
                .estimate_egress_cost_units(bindings.source(), entry.expected_size())
                .await
                .map_err(map_durable_port_error)?;
            let target_cost_units = self
                .target
                .estimate_cost_units(bindings.target(), entry.expected_size())
                .await
                .map_err(map_durable_port_error)?;
            let cost_units = source_cost_units
                .checked_add(target_cost_units)
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
            if source_cost_units == 0 || target_cost_units == 0 {
                return Err(ObjectBackfillErrorV1::InvalidContract);
            }
            if journal
                .usage()
                .admitted_logical_bytes()
                .saturating_add(entry.expected_size().get())
                > policy.max_logical_bytes_per_run()
                || journal
                    .usage()
                    .admitted_cost_units()
                    .saturating_add(cost_units)
                    > policy.max_cost_units_per_run()
            {
                return Ok(BackfillProcessOutcomeV1::BudgetExhausted);
            }
            let operation_id = BackfillOperationIdV1::new();
            let claim_now = self.now_at_or_after(journal.last_transition_at())?;
            let mut next = journal.clone();
            let lease = match next.claim(
                entry_id,
                worker_id,
                operation_id,
                claim_now,
                policy,
                entry.expected_size(),
                cost_units,
            ) {
                Ok(lease) => lease,
                Err(BackfillContractErrorV1::InvalidTransition) => continue,
                Err(error) => return Err(map_contract_error(error)),
            };
            match self
                .cas_exact(manifest_id, journal.revision(), &next)
                .await?
            {
                CasOutcomeV1::Applied => {
                    claimed = Some((entry.clone(), operation_id, lease));
                    break;
                }
                CasOutcomeV1::Conflict => continue,
            }
        }
        let Some((entry, operation_id, lease)) = claimed else {
            return Err(ObjectBackfillErrorV1::StateConflict);
        };

        let transfer = self
            .transfer(&manifest, &entry, operation_id, lease, bindings, policy)
            .await;
        match transfer {
            Ok(success) => {
                self.persist_success(&manifest, &entry, operation_id, lease, success, bindings)
                    .await
            }
            Err(failure) => {
                self.persist_failure(&manifest, entry.entry_id(), lease, failure, policy)
                    .await
            }
        }
    }

    async fn transfer(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
        lease: BackfillLeaseV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<TransferSuccessV1, BackfillFailureClassV1> {
        if let Some(metadata) = self
            .target
            .head(bindings.target(), entry.target_key())
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?
        {
            self.validate_metadata(manifest, entry, &metadata, ObjectSideV1::Target)?;
            let verified = self
                .verify_target(
                    manifest,
                    entry,
                    operation_id,
                    bindings.target(),
                    policy,
                    Some(lease),
                )
                .await?;
            return Ok(TransferSuccessV1 {
                result: BackfillWriteResultV1::ReusedExact,
                destination_version_present: verified.metadata.destination_version().is_some(),
                probe_profile_version: verified.probe.map(|probe| probe.profile_version()),
            });
        }

        let spec = BackfillCreateSpecV1::new(
            entry.entry_id(),
            operation_id,
            manifest.target().authority_fingerprint().clone(),
            entry.tenant_id(),
            entry.video_id(),
            entry.role(),
            entry.target_key().clone(),
            entry.expected_size(),
            entry.strong_sha256().clone(),
            entry.content_type().clone(),
        )
        .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?;

        let opened = self
            .source
            .open_read(bindings.source(), entry.source_reference(), operation_id)
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?;
        let (source_metadata, mut source_body) = opened.into_parts();
        if let Err(failure) =
            self.validate_metadata(manifest, entry, &source_metadata, ObjectSideV1::Source)
        {
            let _ = source_body.cancel().await;
            return Err(failure);
        }

        let create = match self
            .target
            .begin_conditional_create(bindings.target(), &spec)
            .await
        {
            Ok(create) => create,
            Err(error) => {
                let _ = source_body.cancel().await;
                return Err(map_side_port_error(error, ObjectSideV1::Target));
            }
        };
        let mut writer = match create {
            BackfillConditionalCreateV1::AlreadyPresent(metadata) => {
                let _ = source_body.cancel().await;
                self.validate_metadata(manifest, entry, &metadata, ObjectSideV1::Target)?;
                let verified = self
                    .verify_target(
                        manifest,
                        entry,
                        operation_id,
                        bindings.target(),
                        policy,
                        Some(lease),
                    )
                    .await?;
                return Ok(TransferSuccessV1 {
                    result: BackfillWriteResultV1::ReusedExact,
                    destination_version_present: verified.metadata.destination_version().is_some(),
                    probe_profile_version: verified.probe.map(|probe| probe.profile_version()),
                });
            }
            BackfillConditionalCreateV1::Ready(writer) => writer,
        };

        let mut probe = match self.start_probe(entry).await {
            Ok(probe) => probe,
            Err(failure) => {
                let _ = source_body.cancel().await;
                let _ = writer.cancel().await;
                return Err(failure);
            }
        };
        let copy_result = self
            .copy_stream(
                manifest,
                entry,
                &mut *source_body,
                &mut *writer,
                &mut probe,
                lease,
                policy,
            )
            .await;
        let source_probe = match copy_result {
            Ok(receipt) => receipt,
            Err(failure) => {
                cancel_transfer(&mut *source_body, &mut *writer, &mut probe).await;
                return Err(failure);
            }
        };
        drop(source_metadata);

        if self
            .heartbeat(manifest, entry.entry_id(), lease, policy)
            .await
            .is_err()
            || self
                .cancellation
                .canceled(manifest.manifest_id(), entry.entry_id())
        {
            let _ = writer.cancel().await;
            return Err(BackfillFailureClassV1::TransferCanceled);
        }
        let fence = LiveCommitFenceV1 {
            journals: self.journals,
            clock: self.clock,
            manifest,
            entry_id: entry.entry_id(),
            operation_id,
            lease,
        };
        let commit = writer.commit(source_probe.as_ref(), &fence).await;
        let mut result = BackfillWriteResultV1::Created;
        let acknowledged_version = match commit {
            Ok(receipt) if receipt.operation_id() == operation_id => {
                Some(receipt.destination_version().clone())
            }
            Ok(_) => {
                let _ = writer.cancel().await;
                return Err(BackfillFailureClassV1::TargetMetadataMismatch);
            }
            Err(error) => {
                let _ = writer.cancel().await;
                // The mutation may have committed. Exact streaming post-read is authoritative.
                let mapped = map_side_port_error(error, ObjectSideV1::Target);
                match self
                    .verify_target(
                        manifest,
                        entry,
                        operation_id,
                        bindings.target(),
                        policy,
                        Some(lease),
                    )
                    .await
                {
                    Ok(verified) => {
                        result = BackfillWriteResultV1::ReusedExact;
                        verified.metadata.destination_version().cloned()
                    }
                    Err(_) => return Err(mapped),
                }
            }
        };

        let verified = match self
            .verify_target(
                manifest,
                entry,
                operation_id,
                bindings.target(),
                policy,
                Some(lease),
            )
            .await
        {
            Ok(verified) => verified,
            Err(failure) => {
                let _ = writer.cancel().await;
                return Err(failure);
            }
        };
        let observed_version = verified
            .metadata
            .destination_version()
            .ok_or(BackfillFailureClassV1::TargetMetadataMismatch)?;
        if acknowledged_version
            .as_ref()
            .is_some_and(|version| version != observed_version)
        {
            let _ = writer.cancel().await;
            return Err(BackfillFailureClassV1::TargetMetadataMismatch);
        }
        Ok(TransferSuccessV1 {
            result,
            destination_version_present: true,
            probe_profile_version: verified.probe.map(|probe| probe.profile_version()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn copy_stream(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        source: &mut dyn BackfillReadBodyV1,
        writer: &mut dyn BackfillWriteBodyV1,
        probe: &mut Option<Box<dyn BackfillProbeSessionV1>>,
        lease: BackfillLeaseV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<Option<BackfillProbeReceiptV1>, BackfillFailureClassV1> {
        let mut sha = Sha256::new();
        let mut observed = 0_u64;
        loop {
            if self
                .cancellation
                .canceled(manifest.manifest_id(), entry.entry_id())
            {
                return Err(BackfillFailureClassV1::TransferCanceled);
            }
            self.heartbeat(manifest, entry.entry_id(), lease, policy)
                .await
                .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
            let Some(chunk) = source
                .next_chunk()
                .await
                .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?
            else {
                break;
            };
            validate_chunk(&chunk, policy)?;
            let chunk_size =
                u64::try_from(chunk.len()).map_err(|_| BackfillFailureClassV1::OversizedChunk)?;
            observed = observed
                .checked_add(chunk_size)
                .ok_or(BackfillFailureClassV1::ExtraSourceBytes)?;
            if observed > entry.expected_size().get() {
                return Err(BackfillFailureClassV1::ExtraSourceBytes);
            }
            let chunk_now = self
                .now_at_or_after(manifest.created_at())
                .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
            match self
                .throttle
                .admit_bytes(
                    entry.tenant_id(),
                    manifest.source().provider(),
                    manifest.source().region(),
                    manifest.target().provider(),
                    manifest.target().region(),
                    ByteSize::new(chunk_size)
                        .map_err(|_| BackfillFailureClassV1::OversizedChunk)?,
                    policy.max_bandwidth_bytes_per_second(),
                    chunk_now,
                )
                .await
                .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?
            {
                BackfillThrottleDecisionV1::Allowed => {}
                BackfillThrottleDecisionV1::DeferredUntil(_) => {
                    return Err(BackfillFailureClassV1::ProviderThrottled);
                }
            }
            sha.update(chunk.as_bytes());
            if let Some(session) = probe.as_deref_mut() {
                session
                    .observe(chunk.as_bytes())
                    .await
                    .map_err(map_probe_port_error)?;
            }
            writer
                .write_chunk(chunk)
                .await
                .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?;
            self.heartbeat(manifest, entry.entry_id(), lease, policy)
                .await
                .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
        }
        if observed < entry.expected_size().get() {
            return Err(BackfillFailureClassV1::TruncatedSource);
        }
        let digest = digest_from_hasher(sha);
        if &digest != entry.strong_sha256() {
            return Err(BackfillFailureClassV1::SourceChecksumMismatch);
        }
        if self
            .cancellation
            .canceled(manifest.manifest_id(), entry.entry_id())
        {
            return Err(BackfillFailureClassV1::TransferCanceled);
        }
        let receipt = match probe.as_deref_mut() {
            Some(session) => {
                let receipt = session.finish().await.map_err(map_probe_port_error)?;
                if !receipt.playable()
                    || receipt.profile_version() != entry.media_probe().profile_version()
                {
                    return Err(BackfillFailureClassV1::MediaUnplayable);
                }
                Some(receipt)
            }
            None => None,
        };
        source
            .cancel()
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?;
        Ok(receipt)
    }

    async fn verify_target(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
        access: &BackfillProviderAccessV1<'_>,
        policy: BackfillExecutionPolicyV1,
        lease: Option<BackfillLeaseV1>,
    ) -> Result<VerifiedObjectV1, BackfillFailureClassV1> {
        let head = self
            .target
            .head(access, entry.target_key())
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?
            .ok_or(BackfillFailureClassV1::MissingTarget)?;
        self.validate_metadata(manifest, entry, &head, ObjectSideV1::Target)?;
        let opened = self
            .target
            .open_read(access, entry.target_key(), operation_id)
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?;
        let (metadata, mut body) = opened.into_parts();
        if metadata != head {
            let _ = body.cancel().await;
            return Err(BackfillFailureClassV1::TargetMetadataMismatch);
        }
        self.verify_stream(
            manifest,
            entry,
            BackfillOpenedReadV1::new(metadata, body),
            ObjectSideV1::Target,
            policy,
            lease,
        )
        .await
    }

    async fn verify_source(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
        access: &BackfillProviderAccessV1<'_>,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<VerifiedObjectV1, BackfillFailureClassV1> {
        let opened = self
            .source
            .open_read(access, entry.source_reference(), operation_id)
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?;
        self.verify_stream(manifest, entry, opened, ObjectSideV1::Source, policy, None)
            .await
    }

    async fn verify_stream(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        opened: BackfillOpenedReadV1,
        side: ObjectSideV1,
        policy: BackfillExecutionPolicyV1,
        lease: Option<BackfillLeaseV1>,
    ) -> Result<VerifiedObjectV1, BackfillFailureClassV1> {
        let (metadata, mut body) = opened.into_parts();
        if let Err(failure) = self.validate_metadata(manifest, entry, &metadata, side) {
            let _ = body.cancel().await;
            return Err(failure);
        }
        let mut probe = match self.start_probe(entry).await {
            Ok(probe) => probe,
            Err(failure) => {
                let _ = body.cancel().await;
                return Err(failure);
            }
        };
        let mut sha = Sha256::new();
        let mut observed = 0_u64;
        loop {
            if self
                .cancellation
                .canceled(manifest.manifest_id(), entry.entry_id())
            {
                let _ = body.cancel().await;
                if let Some(session) = probe.as_deref_mut() {
                    let _ = session.cancel().await;
                }
                return Err(BackfillFailureClassV1::TransferCanceled);
            }
            if let Some(lease) = lease {
                self.heartbeat(manifest, entry.entry_id(), lease, policy)
                    .await
                    .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
            }
            let chunk = match body.next_chunk().await {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = body.cancel().await;
                    if let Some(session) = probe.as_deref_mut() {
                        let _ = session.cancel().await;
                    }
                    return Err(map_side_port_error(error, side));
                }
            };
            let Some(chunk) = chunk else { break };
            if let Err(failure) = validate_chunk(&chunk, policy) {
                let _ = body.cancel().await;
                if let Some(session) = probe.as_deref_mut() {
                    let _ = session.cancel().await;
                }
                return Err(failure);
            }
            let len =
                u64::try_from(chunk.len()).map_err(|_| BackfillFailureClassV1::OversizedChunk)?;
            observed = observed
                .checked_add(len)
                .ok_or(BackfillFailureClassV1::ExtraSourceBytes)?;
            if observed > entry.expected_size().get() {
                let _ = body.cancel().await;
                if let Some(session) = probe.as_deref_mut() {
                    let _ = session.cancel().await;
                }
                return Err(side.extra_bytes());
            }
            let chunk_now = self
                .now_at_or_after(manifest.created_at())
                .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
            match self
                .throttle
                .admit_bytes(
                    entry.tenant_id(),
                    manifest.source().provider(),
                    manifest.source().region(),
                    manifest.target().provider(),
                    manifest.target().region(),
                    ByteSize::new(len).map_err(|_| BackfillFailureClassV1::OversizedChunk)?,
                    policy.max_bandwidth_bytes_per_second(),
                    chunk_now,
                )
                .await
                .map_err(|error| map_side_port_error(error, side))?
            {
                BackfillThrottleDecisionV1::Allowed => {}
                BackfillThrottleDecisionV1::DeferredUntil(_) => {
                    let _ = body.cancel().await;
                    if let Some(session) = probe.as_deref_mut() {
                        let _ = session.cancel().await;
                    }
                    return Err(BackfillFailureClassV1::ProviderThrottled);
                }
            }
            sha.update(chunk.as_bytes());
            if let Some(session) = probe.as_deref_mut()
                && let Err(error) = session.observe(chunk.as_bytes()).await
            {
                let _ = body.cancel().await;
                let _ = session.cancel().await;
                return Err(map_probe_port_error(error));
            }
        }
        if observed < entry.expected_size().get() {
            let _ = body.cancel().await;
            if let Some(session) = probe.as_deref_mut() {
                let _ = session.cancel().await;
            }
            return Err(side.truncated());
        }
        if digest_from_hasher(sha) != *entry.strong_sha256() {
            let _ = body.cancel().await;
            if let Some(session) = probe.as_deref_mut() {
                let _ = session.cancel().await;
            }
            return Err(side.checksum_mismatch());
        }
        let probe = match probe {
            Some(mut session) => {
                let receipt = match session.finish().await {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        let _ = session.cancel().await;
                        return Err(map_probe_port_error(error));
                    }
                };
                if !receipt.playable()
                    || receipt.profile_version() != entry.media_probe().profile_version()
                {
                    let _ = session.cancel().await;
                    return Err(side.unplayable());
                }
                Some(receipt)
            }
            None => None,
        };
        body.cancel()
            .await
            .map_err(|error| map_side_port_error(error, side))?;
        Ok(VerifiedObjectV1 { metadata, probe })
    }

    async fn start_probe(
        &self,
        entry: &ObjectBackfillManifestEntryV1,
    ) -> Result<Option<Box<dyn BackfillProbeSessionV1>>, BackfillFailureClassV1> {
        if !entry.media_probe().required() {
            return Ok(None);
        }
        self.probes
            .start(entry.role(), entry.media_probe())
            .await
            .map(Some)
            .map_err(map_probe_port_error)
    }

    fn validate_metadata(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        metadata: &BackfillObjectMetadataV1,
        side: ObjectSideV1,
    ) -> Result<(), BackfillFailureClassV1> {
        let authority = match side {
            ObjectSideV1::Source => manifest.source(),
            ObjectSideV1::Target => manifest.target(),
        };
        if metadata.authority_fingerprint() != authority.authority_fingerprint()
            || metadata.owner_tenant() != entry.tenant_id()
            || metadata.video_id() != entry.video_id()
            || metadata.role() != entry.role()
        {
            return Err(side.ownership_mismatch());
        }
        let location_matches = match (side, metadata.location()) {
            (ObjectSideV1::Source, BackfillObjectLocationV1::Source(reference)) => {
                reference == entry.source_reference()
            }
            (ObjectSideV1::Target, BackfillObjectLocationV1::Target(key)) => {
                key == entry.target_key()
            }
            _ => false,
        };
        if !location_matches
            || metadata.logical_bytes() != entry.expected_size()
            || metadata.content_type() != entry.content_type()
            || metadata
                .strong_sha256()
                .is_some_and(|checksum| checksum != entry.strong_sha256())
        {
            return Err(side.metadata_mismatch());
        }
        if matches!(side, ObjectSideV1::Source)
            && entry.source_provider_checksum().is_some()
            && metadata.provider_checksum() != entry.source_provider_checksum()
        {
            return Err(BackfillFailureClassV1::SourceMetadataMismatch);
        }
        if matches!(side, ObjectSideV1::Target) && metadata.destination_version().is_none() {
            return Err(BackfillFailureClassV1::TargetMetadataMismatch);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_success(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
        lease: BackfillLeaseV1,
        success: TransferSuccessV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
    ) -> Result<BackfillProcessOutcomeV1, ObjectBackfillErrorV1> {
        if !success.destination_version_present {
            return Err(ObjectBackfillErrorV1::InvalidContract);
        }
        // A fresh exact HEAD supplies the durable version used by the journal receipt.
        let metadata = self
            .target
            .head(bindings.target(), entry.target_key())
            .await
            .map_err(map_durable_port_error)?
            .ok_or(ObjectBackfillErrorV1::Unavailable)?;
        self.validate_metadata(manifest, entry, &metadata, ObjectSideV1::Target)
            .map_err(|_| ObjectBackfillErrorV1::InvalidContract)?;
        let version = metadata
            .destination_version()
            .cloned()
            .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
        if success.result == BackfillWriteResultV1::Created
            && metadata.operation_id() != Some(operation_id)
        {
            return Err(ObjectBackfillErrorV1::InvalidContract);
        }
        let receipt_now = self.now_at_or_after(manifest.created_at())?;
        let receipt = BackfillOperationReceiptV1::new(
            operation_id,
            entry.entry_id(),
            success.result,
            version,
            entry.expected_size(),
            entry.strong_sha256().clone(),
            success.probe_profile_version,
            receipt_now,
        )
        .map_err(map_contract_error)?;

        for _ in 0..MAX_CAS_ATTEMPTS {
            let completion_now = self.now_at_or_after(receipt_now)?;
            let journal = self.load_journal_at(manifest, completion_now).await?;
            let expected_revision = journal.revision();
            let mut next = journal;
            match next.complete(entry.entry_id(), lease, receipt.clone(), completion_now) {
                Ok(()) => {}
                Err(BackfillContractErrorV1::StaleLease) => {
                    return Ok(BackfillProcessOutcomeV1::LeaseLost(entry.entry_id()));
                }
                Err(error) => return Err(map_contract_error(error)),
            }
            match self
                .cas_exact(manifest.manifest_id(), expected_revision, &next)
                .await?
            {
                CasOutcomeV1::Applied => {
                    return Ok(match success.result {
                        BackfillWriteResultV1::Created => {
                            BackfillProcessOutcomeV1::Copied(entry.entry_id())
                        }
                        BackfillWriteResultV1::ReusedExact | BackfillWriteResultV1::Referenced => {
                            BackfillProcessOutcomeV1::ReusedExact(entry.entry_id())
                        }
                    });
                }
                CasOutcomeV1::Conflict => continue,
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    async fn persist_failure(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        failure: BackfillFailureClassV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<BackfillProcessOutcomeV1, ObjectBackfillErrorV1> {
        for _ in 0..MAX_CAS_ATTEMPTS {
            let failure_now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(manifest, failure_now).await?;
            let expected_revision = journal.revision();
            let mut next = journal;
            match next.fail(entry_id, lease, failure, failure_now, policy) {
                Ok(()) => {}
                Err(BackfillContractErrorV1::StaleLease) => {
                    return Ok(BackfillProcessOutcomeV1::LeaseLost(entry_id));
                }
                Err(error) => return Err(map_contract_error(error)),
            }
            let status = next
                .entry(entry_id)
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?
                .status()
                .clone();
            match self
                .cas_exact(manifest.manifest_id(), expected_revision, &next)
                .await?
            {
                CasOutcomeV1::Applied => {
                    return Ok(match status {
                        BackfillEntryStatusV1::RetryScheduled { .. } => {
                            BackfillProcessOutcomeV1::RetryScheduled { entry_id, failure }
                        }
                        BackfillEntryStatusV1::Quarantined { .. } => {
                            BackfillProcessOutcomeV1::Quarantined { entry_id, failure }
                        }
                        _ => return Err(ObjectBackfillErrorV1::InvalidContract),
                    });
                }
                CasOutcomeV1::Conflict => continue,
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    async fn heartbeat(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), ObjectBackfillErrorV1> {
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            next.renew_lease(entry_id, lease, now, policy.lease_ttl())
                .map_err(map_contract_error)?;
            match self
                .cas_exact(manifest.manifest_id(), revision, &next)
                .await?
            {
                CasOutcomeV1::Applied => return Ok(()),
                CasOutcomeV1::Conflict => continue,
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    async fn recover_or_normalize_expired(
        &self,
        manifest: &ObjectBackfillManifestV1,
        entry: &ObjectBackfillManifestEntryV1,
        operation_id: BackfillOperationIdV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
        observed_expired_at: TimestampMillis,
    ) -> Result<BackfillProcessOutcomeV1, ObjectBackfillErrorV1> {
        let metadata = self
            .target
            .head(bindings.target(), entry.target_key())
            .await
            .map_err(map_durable_port_error)?;
        let verified_recovery = if metadata.as_ref().is_some_and(|metadata| {
            self.validate_metadata(manifest, entry, metadata, ObjectSideV1::Target)
                .is_ok()
                && metadata.operation_id() == Some(operation_id)
        }) {
            self.verify_target(
                manifest,
                entry,
                operation_id,
                bindings.target(),
                manifest.execution_policy(),
                None,
            )
            .await
            .ok()
        } else {
            None
        };
        if let Some(verified) = verified_recovery {
            let version = verified
                .metadata
                .destination_version()
                .cloned()
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
            let receipt_now = self.now_at_or_after(observed_expired_at)?;
            let receipt = BackfillOperationReceiptV1::new(
                operation_id,
                entry.entry_id(),
                BackfillWriteResultV1::Created,
                version,
                entry.expected_size(),
                entry.strong_sha256().clone(),
                verified.probe.map(|probe| probe.profile_version()),
                receipt_now,
            )
            .map_err(map_contract_error)?;
            for _ in 0..MAX_CAS_ATTEMPTS {
                let now = self.now_at_or_after(receipt_now)?;
                let journal = self.load_journal_at(manifest, now).await?;
                let revision = journal.revision();
                let mut next = journal;
                match next.recover_committed(entry.entry_id(), operation_id, receipt.clone(), now) {
                    Ok(()) => {}
                    Err(BackfillContractErrorV1::InvalidTransition) => {
                        return Ok(BackfillProcessOutcomeV1::LeaseLost(entry.entry_id()));
                    }
                    Err(error) => return Err(map_contract_error(error)),
                }
                match self
                    .cas_exact(manifest.manifest_id(), revision, &next)
                    .await?
                {
                    CasOutcomeV1::Applied => {
                        return Ok(BackfillProcessOutcomeV1::ReusedExact(entry.entry_id()));
                    }
                    CasOutcomeV1::Conflict => continue,
                }
            }
            return Err(ObjectBackfillErrorV1::StateConflict);
        }

        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(observed_expired_at)?;
            let journal = self.load_journal_at(manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            match next.normalize_expired_lease(entry.entry_id(), now, manifest.execution_policy()) {
                Ok(()) => {}
                Err(BackfillContractErrorV1::InvalidTransition) => {
                    return Ok(BackfillProcessOutcomeV1::LeaseLost(entry.entry_id()));
                }
                Err(error) => return Err(map_contract_error(error)),
            }
            let status = next
                .entry(entry.entry_id())
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?
                .status()
                .clone();
            match self
                .cas_exact(manifest.manifest_id(), revision, &next)
                .await?
            {
                CasOutcomeV1::Applied => {
                    return Ok(match status {
                        BackfillEntryStatusV1::RetryScheduled { failure, .. } => {
                            BackfillProcessOutcomeV1::RetryScheduled {
                                entry_id: entry.entry_id(),
                                failure,
                            }
                        }
                        BackfillEntryStatusV1::Quarantined { failure, .. } => {
                            BackfillProcessOutcomeV1::Quarantined {
                                entry_id: entry.entry_id(),
                                failure,
                            }
                        }
                        _ => return Err(ObjectBackfillErrorV1::InvalidContract),
                    });
                }
                CasOutcomeV1::Conflict => continue,
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    pub async fn pause(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(&manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            next.pause(now).map_err(map_contract_error)?;
            if self
                .cas_exact(manifest_id, revision, &next)
                .await?
                .applied()
            {
                return Ok(next);
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    pub async fn resume(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(&manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            next.resume(now).map_err(map_contract_error)?;
            if self
                .cas_exact(manifest_id, revision, &next)
                .await?
                .applied()
            {
                return Ok(next);
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    pub async fn abort(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(&manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            next.abort(now).map_err(map_contract_error)?;
            if self
                .cas_exact(manifest_id, revision, &next)
                .await?
                .applied()
            {
                self.cancellation
                    .request_manifest_abort(manifest_id)
                    .await
                    .map_err(map_durable_port_error)?;
                return Ok(next);
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    pub async fn approve_disposition(
        &self,
        manifest_id: BackfillManifestIdV1,
        entry_id: BackfillEntryIdV1,
        disposition: BackfillOwnerDispositionV1,
        capability: &BackfillOwnerApprovalCapabilityV1,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        let entry = manifest
            .entry(entry_id)
            .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(manifest.created_at())?;
            let journal = self.load_journal_at(&manifest, now).await?;
            let approval = self
                .approvals
                .verify_disposition(
                    capability,
                    &manifest,
                    entry_id,
                    entry.tenant_id(),
                    disposition,
                    now,
                )
                .await
                .map_err(map_approval_port_error)?;
            let revision = journal.revision();
            let mut next = journal;
            next.approve_disposition(entry_id, disposition, approval, now)
                .map_err(map_contract_error)?;
            if self
                .cas_exact(manifest_id, revision, &next)
                .await?
                .applied()
            {
                return Ok(next);
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    pub async fn approve_source_release(
        &self,
        manifest_id: BackfillManifestIdV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
        capability: &BackfillOwnerApprovalCapabilityV1,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        let report = self.reconcile(manifest_id, bindings).await?;
        if !report.clean() {
            return Err(ObjectBackfillErrorV1::StateConflict);
        }
        for _ in 0..MAX_CAS_ATTEMPTS {
            let now = self.now_at_or_after(report.generated_at())?;
            report
                .validate_integrity(&manifest, now, reconciliation_max_age())
                .map_err(map_contract_error)?;
            let approval = self
                .approvals
                .verify_source_release(capability, &manifest, &report, now)
                .await
                .map_err(map_approval_port_error)?;
            let journal = self.load_journal_at(&manifest, now).await?;
            let revision = journal.revision();
            let mut next = journal;
            next.approve_source_release(
                &manifest,
                &report,
                approval,
                now,
                reconciliation_max_age(),
            )
            .map_err(map_contract_error)?;
            if self
                .cas_exact(manifest_id, revision, &next)
                .await?
                .applied()
            {
                return Ok(next);
            }
        }
        Err(ObjectBackfillErrorV1::StateConflict)
    }

    /// Independently inventories both providers, then streams every expected object again.
    /// It never calls a mutation port; its repair plan is therefore structurally dry-run only.
    pub async fn reconcile(
        &self,
        manifest_id: BackfillManifestIdV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
    ) -> Result<ObjectBackfillReconciliationReportV1, ObjectBackfillErrorV1> {
        let manifest = self.load_manifest(manifest_id).await?;
        let generated_at = self.now_at_or_after(manifest.created_at())?;
        let policy = manifest.execution_policy();
        self.validate_bindings(&manifest, bindings)?;
        self.preflight(bindings).await?;
        let journal = self.load_journal_at(&manifest, generated_at).await?;

        let mut tenants = manifest
            .entries()
            .iter()
            .map(ObjectBackfillManifestEntryV1::tenant_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        tenants.sort_by_key(ToString::to_string);
        let mut discrepancies = Vec::new();
        let source_inventory = match self.inventory_source(bindings.source(), &tenants).await {
            Ok(objects) => objects,
            Err(_) => {
                discrepancies.push(BackfillDiscrepancyV1::new(
                    None,
                    BackfillInventorySideV1::Source,
                    BackfillDiscrepancyKindV1::ProviderUnavailable,
                    authority_inventory_fingerprint(
                        manifest.source().authority_fingerprint(),
                        BackfillInventorySideV1::Source,
                    ),
                ));
                InventoryIndexV1::default()
            }
        };
        let target_inventory = match self.inventory_target(bindings.target(), &tenants).await {
            Ok(objects) => objects,
            Err(_) => {
                discrepancies.push(BackfillDiscrepancyV1::new(
                    None,
                    BackfillInventorySideV1::Target,
                    BackfillDiscrepancyKindV1::ProviderUnavailable,
                    authority_inventory_fingerprint(
                        manifest.target().authority_fingerprint(),
                        BackfillInventorySideV1::Target,
                    ),
                ));
                InventoryIndexV1::default()
            }
        };

        let mut observed_source = TotalsBuilderV1::default();
        let mut observed_target = TotalsBuilderV1::default();
        let mut expected_target_keys = HashSet::new();
        let mut expected_source_keys = HashSet::new();
        let mut excluded_target_keys = HashSet::new();
        let mut excluded_source_keys = HashSet::new();
        let mut seen_manifest_sources = HashSet::new();
        let mut disposition_by_entry = HashMap::new();
        let mut dispositions = Vec::new();

        for progress in journal.entries() {
            if let BackfillEntryStatusV1::Quarantined {
                failure,
                disposition:
                    disposition @ (BackfillOwnerDispositionV1::ReferenceApproved
                    | BackfillOwnerDispositionV1::ExcludeApproved),
                approval: Some(approval),
            } = progress.status()
            {
                disposition_by_entry.insert(progress.entry_id(), *disposition);
                dispositions.push(
                    BackfillReconciliationDispositionV1::new(
                        progress.entry_id(),
                        *failure,
                        *disposition,
                        approval.clone(),
                    )
                    .map_err(map_contract_error)?,
                );
            }
        }

        for entry in manifest.entries() {
            match disposition_by_entry.get(&entry.entry_id()).copied() {
                Some(BackfillOwnerDispositionV1::ReferenceApproved) => {
                    expected_source_keys.insert(entry.source_reference().as_str().to_owned());
                }
                Some(BackfillOwnerDispositionV1::ExcludeApproved) => {
                    excluded_source_keys.insert(entry.source_reference().as_str().to_owned());
                    excluded_target_keys.insert(entry.target_key().as_str().to_owned());
                }
                Some(
                    BackfillOwnerDispositionV1::PendingOwnerApproval
                    | BackfillOwnerDispositionV1::RetryApproved,
                ) => return Err(ObjectBackfillErrorV1::InvalidContract),
                None => {
                    expected_source_keys.insert(entry.source_reference().as_str().to_owned());
                    expected_target_keys.insert(entry.target_key().as_str().to_owned());
                }
            }
        }
        excluded_source_keys.retain(|key| !expected_source_keys.contains(key));
        excluded_target_keys.retain(|key| !expected_target_keys.contains(key));

        for metadata in source_inventory.all() {
            if !matches!(
                metadata.location(),
                BackfillObjectLocationV1::Source(reference)
                    if excluded_source_keys.contains(reference.as_str())
            ) {
                observed_source.add_observed(metadata);
            }
        }
        for metadata in target_inventory.all() {
            if !matches!(
                metadata.location(),
                BackfillObjectLocationV1::Target(key)
                    if excluded_target_keys.contains(key.as_str())
            ) {
                observed_target.add_observed(metadata);
            }
        }

        for entry in manifest.entries() {
            let progress = journal
                .entry(entry.entry_id())
                .ok_or(ObjectBackfillErrorV1::InvalidContract)?;
            let disposition = disposition_by_entry.get(&entry.entry_id()).copied();
            let excluded = disposition == Some(BackfillOwnerDispositionV1::ExcludeApproved);
            let referenced = disposition == Some(BackfillOwnerDispositionV1::ReferenceApproved);
            let source_fingerprint =
                expected_entry_fingerprint(&manifest, entry, BackfillInventorySideV1::Source);
            let target_fingerprint =
                expected_entry_fingerprint(&manifest, entry, BackfillInventorySideV1::Target);
            if !excluded
                && !seen_manifest_sources.insert(entry.source_reference().as_str().to_owned())
            {
                discrepancies.push(BackfillDiscrepancyV1::new(
                    Some(entry.entry_id()),
                    BackfillInventorySideV1::Source,
                    BackfillDiscrepancyKindV1::DuplicateSource,
                    source_fingerprint.clone(),
                ));
            }

            if !excluded {
                let source_matches = source_inventory.source(entry.source_reference());
                if source_matches.is_empty() {
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        Some(entry.entry_id()),
                        BackfillInventorySideV1::Source,
                        BackfillDiscrepancyKindV1::MissingSource,
                        source_fingerprint.clone(),
                    ));
                } else if source_matches.len() > 1 {
                    for (occurrence, metadata) in source_matches.iter().enumerate() {
                        discrepancies.push(BackfillDiscrepancyV1::new(
                            Some(entry.entry_id()),
                            BackfillInventorySideV1::Source,
                            BackfillDiscrepancyKindV1::DuplicateSource,
                            metadata_fingerprint(
                                metadata,
                                BackfillInventorySideV1::Source,
                                occurrence,
                            ),
                        ));
                    }
                }
                if !source_matches.is_empty() {
                    match self
                        .verify_source(
                            &manifest,
                            entry,
                            BackfillOperationIdV1::new(),
                            bindings.source(),
                            policy,
                        )
                        .await
                    {
                        Ok(verified) => {
                            for (occurrence, metadata) in source_matches.iter().enumerate() {
                                if *metadata == verified.metadata {
                                    observed_source.mark_verified(entry, verified.probe.is_some());
                                } else {
                                    discrepancies.push(BackfillDiscrepancyV1::new(
                                        Some(entry.entry_id()),
                                        BackfillInventorySideV1::Source,
                                        BackfillDiscrepancyKindV1::MetadataDivergence,
                                        metadata_fingerprint(
                                            metadata,
                                            BackfillInventorySideV1::Source,
                                            occurrence,
                                        ),
                                    ));
                                }
                            }
                        }
                        Err(failure) => discrepancies.push(discrepancy_from_failure(
                            entry.entry_id(),
                            ObjectSideV1::Source,
                            failure,
                            source_fingerprint.clone(),
                        )),
                    }
                }
            }

            if !excluded && !referenced {
                let target_matches = target_inventory.target(entry.target_key());
                let mut target_verified = false;
                let mut verified_metadata = None;
                if target_matches.is_empty() {
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        Some(entry.entry_id()),
                        BackfillInventorySideV1::Target,
                        BackfillDiscrepancyKindV1::MissingTarget,
                        target_fingerprint.clone(),
                    ));
                } else if target_matches.len() > 1 {
                    for (occurrence, metadata) in target_matches.iter().enumerate() {
                        discrepancies.push(BackfillDiscrepancyV1::new(
                            Some(entry.entry_id()),
                            BackfillInventorySideV1::Target,
                            BackfillDiscrepancyKindV1::DuplicateTarget,
                            metadata_fingerprint(
                                metadata,
                                BackfillInventorySideV1::Target,
                                occurrence,
                            ),
                        ));
                    }
                }
                if !target_matches.is_empty() {
                    match self
                        .verify_target(
                            &manifest,
                            entry,
                            BackfillOperationIdV1::new(),
                            bindings.target(),
                            policy,
                            None,
                        )
                        .await
                    {
                        Ok(verified) => {
                            for (occurrence, metadata) in target_matches.iter().enumerate() {
                                if *metadata == verified.metadata {
                                    observed_target.mark_verified(entry, verified.probe.is_some());
                                    target_verified = true;
                                } else {
                                    discrepancies.push(BackfillDiscrepancyV1::new(
                                        Some(entry.entry_id()),
                                        BackfillInventorySideV1::Target,
                                        BackfillDiscrepancyKindV1::MetadataDivergence,
                                        metadata_fingerprint(
                                            metadata,
                                            BackfillInventorySideV1::Target,
                                            occurrence,
                                        ),
                                    ));
                                }
                            }
                            verified_metadata = Some(verified.metadata);
                        }
                        Err(failure) => discrepancies.push(discrepancy_from_failure(
                            entry.entry_id(),
                            ObjectSideV1::Target,
                            failure,
                            target_fingerprint.clone(),
                        )),
                    }
                }
                let checkpoint_exact = match (progress.status(), verified_metadata.as_ref()) {
                    (BackfillEntryStatusV1::Succeeded { receipt }, Some(metadata)) => {
                        metadata.destination_version() == Some(receipt.destination_version())
                            && receipt.logical_bytes() == entry.expected_size()
                            && receipt.strong_sha256() == entry.strong_sha256()
                            && match receipt.result() {
                                BackfillWriteResultV1::Created => {
                                    metadata.operation_id() == Some(receipt.operation_id())
                                }
                                BackfillWriteResultV1::ReusedExact => true,
                                BackfillWriteResultV1::Referenced => false,
                            }
                    }
                    (BackfillEntryStatusV1::Succeeded { .. }, None) => false,
                    (_, _) => !target_verified,
                };
                if !checkpoint_exact {
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        Some(entry.entry_id()),
                        BackfillInventorySideV1::Target,
                        BackfillDiscrepancyKindV1::CheckpointDivergence,
                        target_fingerprint,
                    ));
                }
            }
        }

        for metadata in source_inventory.all() {
            if let BackfillObjectLocationV1::Source(reference) = metadata.location()
                && !expected_source_keys.contains(reference.as_str())
                && !excluded_source_keys.contains(reference.as_str())
            {
                let fingerprint =
                    metadata_fingerprint(metadata, BackfillInventorySideV1::Source, 0);
                if self
                    .verify_inventory_source(&manifest, bindings.source(), metadata, policy)
                    .await
                    .is_err()
                {
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        None,
                        BackfillInventorySideV1::Source,
                        BackfillDiscrepancyKindV1::MetadataDivergence,
                        fingerprint.clone(),
                    ));
                }
                discrepancies.push(BackfillDiscrepancyV1::new(
                    None,
                    BackfillInventorySideV1::Source,
                    BackfillDiscrepancyKindV1::OrphanSource,
                    fingerprint,
                ));
            }
        }

        for metadata in target_inventory.all() {
            match metadata.location() {
                BackfillObjectLocationV1::Target(key)
                    if !expected_target_keys.contains(key.as_str())
                        && !excluded_target_keys.contains(key.as_str()) =>
                {
                    let fingerprint =
                        metadata_fingerprint(metadata, BackfillInventorySideV1::Target, 0);
                    if self
                        .verify_inventory_target(&manifest, bindings.target(), metadata, policy)
                        .await
                        .is_err()
                    {
                        discrepancies.push(BackfillDiscrepancyV1::new(
                            None,
                            BackfillInventorySideV1::Target,
                            BackfillDiscrepancyKindV1::MetadataDivergence,
                            fingerprint.clone(),
                        ));
                    }
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        None,
                        BackfillInventorySideV1::Target,
                        BackfillDiscrepancyKindV1::OrphanTarget,
                        fingerprint,
                    ));
                }
                BackfillObjectLocationV1::Source(_) => {
                    discrepancies.push(BackfillDiscrepancyV1::new(
                        None,
                        BackfillInventorySideV1::Target,
                        BackfillDiscrepancyKindV1::MetadataDivergence,
                        metadata_fingerprint(metadata, BackfillInventorySideV1::Target, 0),
                    ));
                }
                _ => {}
            }
        }

        ObjectBackfillReconciliationReportV1::new_with_dispositions(
            &manifest,
            generated_at,
            observed_source.finish(),
            observed_target.finish(),
            dispositions,
            discrepancies,
        )
        .map_err(map_contract_error)
    }

    async fn verify_inventory_source(
        &self,
        manifest: &ObjectBackfillManifestV1,
        access: &BackfillProviderAccessV1<'_>,
        expected: &BackfillObjectMetadataV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), BackfillFailureClassV1> {
        let BackfillObjectLocationV1::Source(reference) = expected.location() else {
            return Err(BackfillFailureClassV1::SourceMetadataMismatch);
        };
        let opened = self
            .source
            .open_read(access, reference, BackfillOperationIdV1::new())
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Source))?;
        self.verify_inventory_stream(manifest, expected, opened, ObjectSideV1::Source, policy)
            .await
    }

    async fn verify_inventory_target(
        &self,
        manifest: &ObjectBackfillManifestV1,
        access: &BackfillProviderAccessV1<'_>,
        expected: &BackfillObjectMetadataV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), BackfillFailureClassV1> {
        let BackfillObjectLocationV1::Target(key) = expected.location() else {
            return Err(BackfillFailureClassV1::TargetMetadataMismatch);
        };
        let head = self
            .target
            .head(access, key)
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?
            .ok_or(BackfillFailureClassV1::MissingTarget)?;
        if &head != expected {
            return Err(BackfillFailureClassV1::TargetMetadataMismatch);
        }
        let opened = self
            .target
            .open_read(access, key, BackfillOperationIdV1::new())
            .await
            .map_err(|error| map_side_port_error(error, ObjectSideV1::Target))?;
        self.verify_inventory_stream(manifest, expected, opened, ObjectSideV1::Target, policy)
            .await
    }

    async fn verify_inventory_stream(
        &self,
        manifest: &ObjectBackfillManifestV1,
        expected: &BackfillObjectMetadataV1,
        opened: BackfillOpenedReadV1,
        side: ObjectSideV1,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), BackfillFailureClassV1> {
        let (metadata, mut body) = opened.into_parts();
        if &metadata != expected {
            let _ = body.cancel().await;
            return Err(side.metadata_mismatch());
        }
        let Some(expected_sha) = expected.strong_sha256() else {
            let _ = body.cancel().await;
            return Err(side.metadata_mismatch());
        };
        let mut sha = Sha256::new();
        let mut observed = 0_u64;
        loop {
            let chunk = match body.next_chunk().await {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = body.cancel().await;
                    return Err(map_side_port_error(error, side));
                }
            };
            let Some(chunk) = chunk else { break };
            if let Err(failure) = validate_chunk(&chunk, policy) {
                let _ = body.cancel().await;
                return Err(failure);
            }
            let bytes =
                u64::try_from(chunk.len()).map_err(|_| BackfillFailureClassV1::OversizedChunk)?;
            observed = observed
                .checked_add(bytes)
                .ok_or_else(|| side.extra_bytes())?;
            if observed > expected.logical_bytes().get() {
                let _ = body.cancel().await;
                return Err(side.extra_bytes());
            }
            let now = self
                .now_at_or_after(manifest.created_at())
                .map_err(|_| BackfillFailureClassV1::TransferCanceled)?;
            if !matches!(
                self.throttle
                    .admit_bytes(
                        expected.owner_tenant(),
                        manifest.source().provider(),
                        manifest.source().region(),
                        manifest.target().provider(),
                        manifest.target().region(),
                        ByteSize::new(bytes).map_err(|_| BackfillFailureClassV1::OversizedChunk)?,
                        policy.max_bandwidth_bytes_per_second(),
                        now,
                    )
                    .await
                    .map_err(|error| map_side_port_error(error, side))?,
                BackfillThrottleDecisionV1::Allowed
            ) {
                let _ = body.cancel().await;
                return Err(BackfillFailureClassV1::ProviderThrottled);
            }
            sha.update(chunk.as_bytes());
        }
        body.cancel()
            .await
            .map_err(|error| map_side_port_error(error, side))?;
        if observed != expected.logical_bytes().get() || digest_from_hasher(sha) != *expected_sha {
            return Err(side.checksum_mismatch());
        }
        Ok(())
    }

    async fn inventory_source(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenants: &[TenantId],
    ) -> Result<InventoryIndexV1, BackfillPortErrorV1> {
        let first = self.collect_source_inventory(access, tenants).await?;
        let second = self.collect_source_inventory(access, tenants).await?;
        if first != second {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(first)
    }

    async fn collect_source_inventory(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenants: &[TenantId],
    ) -> Result<InventoryIndexV1, BackfillPortErrorV1> {
        let mut index = InventoryIndexV1::default();
        for tenant in tenants {
            let mut cursor: Option<BackfillInventoryCursorV1> = None;
            let mut seen = HashSet::new();
            let mut expected_page_index = 0_u64;
            let mut snapshot = None;
            let mut terminal = false;
            for _ in 0..MAX_INVENTORY_PAGES {
                let page = self
                    .source
                    .inventory_page(access, *tenant, cursor.as_ref(), INVENTORY_PAGE_SIZE)
                    .await?;
                let (page_objects, next, page_snapshot, page_index) = page.into_parts();
                if page_index != expected_page_index
                    || snapshot
                        .as_ref()
                        .is_some_and(|digest| digest != &page_snapshot)
                    || page_objects.iter().any(|metadata| {
                        !inventory_row_in_scope(
                            access,
                            *tenant,
                            metadata,
                            BackfillInventorySideV1::Source,
                        )
                    })
                {
                    return Err(BackfillPortErrorV1::InvalidResponse);
                }
                snapshot.get_or_insert(page_snapshot.clone());
                for metadata in page_objects {
                    index.insert(metadata)?;
                }
                let Some(next) = next else {
                    terminal = true;
                    break;
                };
                if !seen.insert(next.expose_to_adapter().to_owned()) {
                    return Err(BackfillPortErrorV1::InvalidResponse);
                }
                cursor = Some(next);
                expected_page_index = expected_page_index.saturating_add(1);
            }
            if !terminal {
                return Err(BackfillPortErrorV1::InvalidResponse);
            }
            index.record_snapshot(
                *tenant,
                snapshot.ok_or(BackfillPortErrorV1::InvalidResponse)?,
            )?;
        }
        Ok(index)
    }

    async fn inventory_target(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenants: &[TenantId],
    ) -> Result<InventoryIndexV1, BackfillPortErrorV1> {
        let first = self.collect_target_inventory(access, tenants).await?;
        let second = self.collect_target_inventory(access, tenants).await?;
        if first != second {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(first)
    }

    async fn collect_target_inventory(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenants: &[TenantId],
    ) -> Result<InventoryIndexV1, BackfillPortErrorV1> {
        let mut index = InventoryIndexV1::default();
        for tenant in tenants {
            let mut cursor: Option<BackfillInventoryCursorV1> = None;
            let mut seen = HashSet::new();
            let mut expected_page_index = 0_u64;
            let mut snapshot = None;
            let mut terminal = false;
            for _ in 0..MAX_INVENTORY_PAGES {
                let page = self
                    .target
                    .inventory_page(access, *tenant, cursor.as_ref(), INVENTORY_PAGE_SIZE)
                    .await?;
                let (page_objects, next, page_snapshot, page_index) = page.into_parts();
                if page_index != expected_page_index
                    || snapshot
                        .as_ref()
                        .is_some_and(|digest| digest != &page_snapshot)
                    || page_objects.iter().any(|metadata| {
                        !inventory_row_in_scope(
                            access,
                            *tenant,
                            metadata,
                            BackfillInventorySideV1::Target,
                        )
                    })
                {
                    return Err(BackfillPortErrorV1::InvalidResponse);
                }
                snapshot.get_or_insert(page_snapshot.clone());
                for metadata in page_objects {
                    index.insert(metadata)?;
                }
                let Some(next) = next else {
                    terminal = true;
                    break;
                };
                if !seen.insert(next.expose_to_adapter().to_owned()) {
                    return Err(BackfillPortErrorV1::InvalidResponse);
                }
                cursor = Some(next);
                expected_page_index = expected_page_index.saturating_add(1);
            }
            if !terminal {
                return Err(BackfillPortErrorV1::InvalidResponse);
            }
            index.record_snapshot(
                *tenant,
                snapshot.ok_or(BackfillPortErrorV1::InvalidResponse)?,
            )?;
        }
        Ok(index)
    }

    async fn load_manifest(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<ObjectBackfillManifestV1, ObjectBackfillErrorV1> {
        let manifest = self
            .manifests
            .load(manifest_id)
            .await
            .map_err(map_durable_port_error)?
            .ok_or(ObjectBackfillErrorV1::ManifestNotFound)?;
        manifest.validate_integrity().map_err(map_contract_error)?;
        Ok(manifest)
    }

    async fn load_journal_at(
        &self,
        manifest: &ObjectBackfillManifestV1,
        now: TimestampMillis,
    ) -> Result<ObjectBackfillJournalV1, ObjectBackfillErrorV1> {
        let journal = self
            .journals
            .load(manifest.manifest_id())
            .await
            .map_err(map_durable_port_error)?
            .ok_or(ObjectBackfillErrorV1::JournalNotFound)?;
        journal
            .validate_for_manifest_at(manifest, now)
            .map_err(map_contract_error)?;
        Ok(journal)
    }

    fn now_at_or_after(
        &self,
        minimum: TimestampMillis,
    ) -> Result<TimestampMillis, ObjectBackfillErrorV1> {
        let now = self.clock.now().map_err(map_durable_port_error)?;
        if now < minimum {
            return Err(ObjectBackfillErrorV1::ClockRollback);
        }
        Ok(now)
    }

    fn validate_bindings(
        &self,
        manifest: &ObjectBackfillManifestV1,
        bindings: &BackfillRuntimeBindingsV1<'_>,
    ) -> Result<(), ObjectBackfillErrorV1> {
        if bindings.source().authority() != manifest.source()
            || bindings.target().authority() != manifest.target()
        {
            return Err(ObjectBackfillErrorV1::AuthorityMismatch);
        }
        Ok(())
    }

    async fn preflight(
        &self,
        bindings: &BackfillRuntimeBindingsV1<'_>,
    ) -> Result<(), ObjectBackfillErrorV1> {
        let source = self
            .source
            .capabilities(bindings.source())
            .await
            .map_err(map_durable_port_error)?;
        let target = self
            .target
            .capabilities(bindings.target())
            .await
            .map_err(map_durable_port_error)?;
        if !source.source_satisfies_backfill() || !target.target_satisfies_backfill() {
            return Err(ObjectBackfillErrorV1::CapabilityMissing);
        }
        Ok(())
    }

    async fn cas_exact(
        &self,
        manifest_id: BackfillManifestIdV1,
        expected_revision: u64,
        next: &ObjectBackfillJournalV1,
    ) -> Result<CasOutcomeV1, ObjectBackfillErrorV1> {
        match self
            .journals
            .compare_and_swap(manifest_id, expected_revision, next)
            .await
        {
            Ok(()) => Ok(CasOutcomeV1::Applied),
            Err(BackfillPortErrorV1::Conflict) => Ok(CasOutcomeV1::Conflict),
            Err(BackfillPortErrorV1::ProviderOutage) => {
                // Reconcile an ambiguous durable commit by exact post-read.
                let observed = self
                    .journals
                    .load(manifest_id)
                    .await
                    .map_err(map_durable_port_error)?;
                if observed.as_ref() == Some(next) {
                    Ok(CasOutcomeV1::Applied)
                } else if observed
                    .as_ref()
                    .is_some_and(|current| current.revision() != expected_revision)
                {
                    Ok(CasOutcomeV1::Conflict)
                } else {
                    Err(ObjectBackfillErrorV1::Unavailable)
                }
            }
            Err(error) => Err(map_durable_port_error(error)),
        }
    }
}

impl std::fmt::Debug for ObjectBackfillCoordinatorV1<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObjectBackfillCoordinatorV1")
            .finish_non_exhaustive()
    }
}

struct LiveCommitFenceV1<'a> {
    journals: &'a dyn BackfillJournalPortV1,
    clock: &'a dyn BackfillClockPortV1,
    manifest: &'a ObjectBackfillManifestV1,
    entry_id: BackfillEntryIdV1,
    operation_id: BackfillOperationIdV1,
    lease: BackfillLeaseV1,
}

#[async_trait::async_trait]
impl BackfillCommitFenceV1 for LiveCommitFenceV1<'_> {
    fn manifest_id(&self) -> BackfillManifestIdV1 {
        self.manifest.manifest_id()
    }

    fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    fn operation_id(&self) -> BackfillOperationIdV1 {
        self.operation_id
    }

    fn lease(&self) -> BackfillLeaseV1 {
        self.lease
    }

    async fn authorize_publication(&self) -> Result<(), BackfillPortErrorV1> {
        let now = self.clock.now()?;
        if now < self.manifest.created_at() {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        let journal = self
            .journals
            .load(self.manifest.manifest_id())
            .await?
            .ok_or(BackfillPortErrorV1::NotFound)?;
        journal
            .validate_for_manifest_at(self.manifest, now)
            .map_err(|_| BackfillPortErrorV1::InvalidResponse)?;
        if !journal.publication_allowed(self.entry_id, self.operation_id, self.lease, now) {
            return Err(BackfillPortErrorV1::Canceled);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasOutcomeV1 {
    Applied,
    Conflict,
}

impl CasOutcomeV1 {
    const fn applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

#[derive(Debug, Clone, Copy)]
enum ObjectSideV1 {
    Source,
    Target,
}

impl ObjectSideV1 {
    const fn missing(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::MissingSource,
            Self::Target => BackfillFailureClassV1::MissingTarget,
        }
    }

    const fn ownership_mismatch(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::SourceOwnershipMismatch,
            Self::Target => BackfillFailureClassV1::TargetOwnershipMismatch,
        }
    }

    const fn metadata_mismatch(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::SourceMetadataMismatch,
            Self::Target => BackfillFailureClassV1::TargetMetadataMismatch,
        }
    }

    const fn truncated(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::TruncatedSource,
            Self::Target => BackfillFailureClassV1::TargetChecksumMismatch,
        }
    }

    const fn extra_bytes(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::ExtraSourceBytes,
            Self::Target => BackfillFailureClassV1::TargetChecksumMismatch,
        }
    }

    const fn checksum_mismatch(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::SourceChecksumMismatch,
            Self::Target => BackfillFailureClassV1::TargetChecksumMismatch,
        }
    }

    const fn unplayable(self) -> BackfillFailureClassV1 {
        match self {
            Self::Source => BackfillFailureClassV1::MediaUnplayable,
            Self::Target => BackfillFailureClassV1::MediaUnplayable,
        }
    }
}

#[derive(Default)]
struct TotalsBuilderV1 {
    object_count: u64,
    logical_bytes: u64,
    role_counts: [u64; 10],
    strong_checksums_verified: u64,
    media_probes_verified: u64,
}

impl TotalsBuilderV1 {
    fn add_observed(&mut self, metadata: &BackfillObjectMetadataV1) {
        self.object_count = self.object_count.saturating_add(1);
        self.logical_bytes = self
            .logical_bytes
            .saturating_add(metadata.logical_bytes().get());
        self.role_counts[role_index(metadata.role())] =
            self.role_counts[role_index(metadata.role())].saturating_add(1);
    }

    fn mark_verified(&mut self, _entry: &ObjectBackfillManifestEntryV1, probed: bool) {
        self.strong_checksums_verified = self.strong_checksums_verified.saturating_add(1);
        if probed {
            // Expected totals carry the required-probe count; observed totals count completions.
            self.media_probes_verified = self.media_probes_verified.saturating_add(1);
        }
    }

    const fn finish(self) -> BackfillInventoryTotalsV1 {
        BackfillInventoryTotalsV1::new(
            self.object_count,
            self.logical_bytes,
            self.role_counts,
            self.strong_checksums_verified,
            self.media_probes_verified,
        )
    }
}

const fn role_index(role: ObjectRole) -> usize {
    match role {
        ObjectRole::Source => 0,
        ObjectRole::RecordingSegment => 1,
        ObjectRole::Thumbnail => 2,
        ObjectRole::Screenshot => 3,
        ObjectRole::Preview => 4,
        ObjectRole::Spritesheet => 5,
        ObjectRole::Audio => 6,
        ObjectRole::Caption => 7,
        ObjectRole::Export => 8,
        ObjectRole::Manifest => 9,
    }
}

fn validate_chunk(
    chunk: &BackfillChunkV1,
    policy: BackfillExecutionPolicyV1,
) -> Result<(), BackfillFailureClassV1> {
    if chunk.is_empty() {
        return Err(BackfillFailureClassV1::EmptyChunk);
    }
    let size = u64::try_from(chunk.len()).map_err(|_| BackfillFailureClassV1::OversizedChunk)?;
    if size > policy.max_chunk_bytes().get() {
        return Err(BackfillFailureClassV1::OversizedChunk);
    }
    Ok(())
}

fn metadata_sort_key(metadata: &BackfillObjectMetadataV1) -> String {
    match metadata.location() {
        BackfillObjectLocationV1::Source(reference) => format!("source:{}", reference.as_str()),
        BackfillObjectLocationV1::Target(key) => format!("target:{}", key.as_str()),
    }
}

fn inventory_row_in_scope(
    access: &BackfillProviderAccessV1<'_>,
    tenant: TenantId,
    metadata: &BackfillObjectMetadataV1,
    side: BackfillInventorySideV1,
) -> bool {
    metadata.authority_fingerprint() == access.authority().authority_fingerprint()
        && metadata.owner_tenant() == tenant
        && matches!(
            (side, metadata.location()),
            (
                BackfillInventorySideV1::Source,
                BackfillObjectLocationV1::Source(_)
            ) | (
                BackfillInventorySideV1::Target,
                BackfillObjectLocationV1::Target(_)
            )
        )
}

fn authority_inventory_fingerprint(
    authority_fingerprint: &ChecksumSha256,
    side: BackfillInventorySideV1,
) -> BackfillObjectFingerprintV1 {
    BackfillObjectFingerprintV1::derive(authority_fingerprint, side, "inventory-scope", None)
}

fn expected_entry_fingerprint(
    manifest: &ObjectBackfillManifestV1,
    entry: &ObjectBackfillManifestEntryV1,
    side: BackfillInventorySideV1,
) -> BackfillObjectFingerprintV1 {
    match side {
        BackfillInventorySideV1::Source => BackfillObjectFingerprintV1::derive(
            manifest.source().authority_fingerprint(),
            side,
            entry.source_reference().as_str(),
            None,
        ),
        BackfillInventorySideV1::Target => BackfillObjectFingerprintV1::derive(
            manifest.target().authority_fingerprint(),
            side,
            entry.target_key().as_str(),
            Some(&entry.target_version().get().to_string()),
        ),
    }
}

fn metadata_fingerprint(
    metadata: &BackfillObjectMetadataV1,
    side: BackfillInventorySideV1,
    occurrence: usize,
) -> BackfillObjectFingerprintV1 {
    let location = match metadata.location() {
        BackfillObjectLocationV1::Source(reference) => reference.as_str(),
        BackfillObjectLocationV1::Target(key) => key.as_str(),
    };
    let occurrence_location = format!("{location}#occurrence-{occurrence}");
    BackfillObjectFingerprintV1::derive(
        metadata.authority_fingerprint(),
        side,
        &occurrence_location,
        metadata
            .destination_version()
            .map(frame_domain::BackfillDestinationVersionV1::as_str),
    )
}

fn digest_from_hasher(hasher: Sha256) -> ChecksumSha256 {
    let bytes = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(64);
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    ChecksumSha256::parse(value).expect("SHA-256 output is always valid lowercase hexadecimal")
}

async fn cancel_transfer(
    source: &mut dyn BackfillReadBodyV1,
    writer: &mut dyn BackfillWriteBodyV1,
    probe: &mut Option<Box<dyn BackfillProbeSessionV1>>,
) {
    let _ = source.cancel().await;
    let _ = writer.cancel().await;
    if let Some(probe) = probe.as_deref_mut() {
        let _ = probe.cancel().await;
    }
}

fn map_side_port_error(error: BackfillPortErrorV1, side: ObjectSideV1) -> BackfillFailureClassV1 {
    match error {
        BackfillPortErrorV1::NotFound => side.missing(),
        BackfillPortErrorV1::Conflict => match side {
            ObjectSideV1::Source => BackfillFailureClassV1::SourceMetadataMismatch,
            ObjectSideV1::Target => BackfillFailureClassV1::DestinationConflict,
        },
        BackfillPortErrorV1::Throttled => BackfillFailureClassV1::ProviderThrottled,
        BackfillPortErrorV1::ExpiredAuthorization => {
            BackfillFailureClassV1::ProviderExpiredAuthorization
        }
        BackfillPortErrorV1::ProviderOutage => BackfillFailureClassV1::ProviderOutage,
        BackfillPortErrorV1::Canceled => BackfillFailureClassV1::TransferCanceled,
        BackfillPortErrorV1::InvalidResponse => side.metadata_mismatch(),
        BackfillPortErrorV1::Unsupported => BackfillFailureClassV1::CapabilityMissing,
    }
}

fn map_approval_port_error(error: BackfillPortErrorV1) -> ObjectBackfillErrorV1 {
    match error {
        BackfillPortErrorV1::ProviderOutage | BackfillPortErrorV1::Throttled => {
            ObjectBackfillErrorV1::Unavailable
        }
        BackfillPortErrorV1::NotFound
        | BackfillPortErrorV1::Conflict
        | BackfillPortErrorV1::ExpiredAuthorization
        | BackfillPortErrorV1::Canceled
        | BackfillPortErrorV1::InvalidResponse
        | BackfillPortErrorV1::Unsupported => ObjectBackfillErrorV1::ApprovalDenied,
    }
}

fn map_probe_port_error(error: BackfillPortErrorV1) -> BackfillFailureClassV1 {
    match error {
        BackfillPortErrorV1::ProviderOutage => BackfillFailureClassV1::ProviderOutage,
        BackfillPortErrorV1::Throttled => BackfillFailureClassV1::ProviderThrottled,
        BackfillPortErrorV1::ExpiredAuthorization => {
            BackfillFailureClassV1::ProviderExpiredAuthorization
        }
        BackfillPortErrorV1::Canceled => BackfillFailureClassV1::TransferCanceled,
        _ => BackfillFailureClassV1::MediaUnplayable,
    }
}

fn map_durable_port_error(error: BackfillPortErrorV1) -> ObjectBackfillErrorV1 {
    match error {
        BackfillPortErrorV1::Conflict => ObjectBackfillErrorV1::StateConflict,
        BackfillPortErrorV1::Unsupported => ObjectBackfillErrorV1::CapabilityMissing,
        BackfillPortErrorV1::InvalidResponse => ObjectBackfillErrorV1::InvalidContract,
        BackfillPortErrorV1::NotFound => ObjectBackfillErrorV1::JournalNotFound,
        BackfillPortErrorV1::Throttled
        | BackfillPortErrorV1::ExpiredAuthorization
        | BackfillPortErrorV1::ProviderOutage
        | BackfillPortErrorV1::Canceled => ObjectBackfillErrorV1::Unavailable,
    }
}

fn map_contract_error(error: BackfillContractErrorV1) -> ObjectBackfillErrorV1 {
    match error {
        BackfillContractErrorV1::StaleLease => ObjectBackfillErrorV1::LeaseLost,
        BackfillContractErrorV1::ClockRollback => ObjectBackfillErrorV1::ClockRollback,
        BackfillContractErrorV1::InvalidApproval => ObjectBackfillErrorV1::ApprovalDenied,
        BackfillContractErrorV1::InvalidTransition
        | BackfillContractErrorV1::InvalidDisposition
        | BackfillContractErrorV1::RetentionBlocked => ObjectBackfillErrorV1::StateConflict,
        BackfillContractErrorV1::UnsupportedVersion
        | BackfillContractErrorV1::InvalidManifest
        | BackfillContractErrorV1::InvalidPolicy
        | BackfillContractErrorV1::InvalidJournal
        | BackfillContractErrorV1::InvalidReport
        | BackfillContractErrorV1::InvalidProviderValue
        | BackfillContractErrorV1::InvalidCredentialReference => {
            ObjectBackfillErrorV1::InvalidContract
        }
    }
}

fn reconciliation_max_age() -> DurationMillis {
    DurationMillis::new(RECONCILIATION_MAX_AGE_MILLIS)
        .expect("reconciliation maximum age is a positive wire-safe constant")
}

fn discrepancy_from_failure(
    entry_id: BackfillEntryIdV1,
    side: ObjectSideV1,
    failure: BackfillFailureClassV1,
    object_fingerprint: BackfillObjectFingerprintV1,
) -> BackfillDiscrepancyV1 {
    let inventory_side = match side {
        ObjectSideV1::Source => BackfillInventorySideV1::Source,
        ObjectSideV1::Target => BackfillInventorySideV1::Target,
    };
    let kind = match failure {
        BackfillFailureClassV1::MissingSource => BackfillDiscrepancyKindV1::MissingSource,
        BackfillFailureClassV1::MissingTarget => BackfillDiscrepancyKindV1::MissingTarget,
        BackfillFailureClassV1::SourceOwnershipMismatch
        | BackfillFailureClassV1::TargetOwnershipMismatch => {
            BackfillDiscrepancyKindV1::OwnershipMismatch
        }
        BackfillFailureClassV1::SourceChecksumMismatch
        | BackfillFailureClassV1::TruncatedSource
        | BackfillFailureClassV1::ExtraSourceBytes
        | BackfillFailureClassV1::EmptyChunk
        | BackfillFailureClassV1::OversizedChunk => BackfillDiscrepancyKindV1::CorruptSource,
        BackfillFailureClassV1::TargetChecksumMismatch
        | BackfillFailureClassV1::DestinationConflict => BackfillDiscrepancyKindV1::CorruptTarget,
        BackfillFailureClassV1::MediaUnplayable => match side {
            ObjectSideV1::Source => BackfillDiscrepancyKindV1::UnplayableSource,
            ObjectSideV1::Target => BackfillDiscrepancyKindV1::UnplayableTarget,
        },
        BackfillFailureClassV1::SourceMetadataMismatch
        | BackfillFailureClassV1::TargetMetadataMismatch => {
            BackfillDiscrepancyKindV1::MetadataDivergence
        }
        BackfillFailureClassV1::CheckpointDivergence => {
            BackfillDiscrepancyKindV1::CheckpointDivergence
        }
        BackfillFailureClassV1::ProviderThrottled
        | BackfillFailureClassV1::ProviderExpiredAuthorization
        | BackfillFailureClassV1::ProviderOutage
        | BackfillFailureClassV1::CircuitOpen
        | BackfillFailureClassV1::TransferCanceled
        | BackfillFailureClassV1::CapabilityMissing
        | BackfillFailureClassV1::BudgetExceeded => BackfillDiscrepancyKindV1::ProviderUnavailable,
    };
    BackfillDiscrepancyV1::new(Some(entry_id), inventory_side, kind, object_fingerprint)
}
