use async_trait::async_trait;
use frame_domain::{
    CacheInvalidationPlan, CachePurgeReceipt, ChecksumSha256, CorrelationId,
    DeletionEvidenceReceipt, DeletionGuardSnapshot, DeletionStage, DeletionWorkflow,
    DurableGovernanceAuditRecord, GovernedObject, GovernedObjectId, LifecycleInventory,
    SignedGrantId, SignedGrantSecret, SignedObjectGrant, StorageQuotaReservation,
    StorageQuotaSnapshot, TenantId, TimestampMillis, VerifiedCustomDomain,
};

use crate::PortError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageGovernanceContextV1 {
    tenant_id: TenantId,
    correlation_id: CorrelationId,
    principal_digest: ChecksumSha256,
}

impl StorageGovernanceContextV1 {
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        correlation_id: CorrelationId,
        principal_digest: ChecksumSha256,
    ) -> Self {
        Self {
            tenant_id,
            correlation_id,
            principal_digest,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    #[must_use]
    pub fn principal_digest(&self) -> &ChecksumSha256 {
        &self.principal_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageCasOutcomeV1 {
    Applied,
    Replay,
    Conflict,
}

#[async_trait]
pub trait StorageGovernanceRepositoryV1: Send + Sync {
    async fn governed_object(
        &self,
        context: StorageGovernanceContextV1,
        object_id: &GovernedObjectId,
    ) -> Result<Option<GovernedObject>, PortError>;

    async fn insert_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant: SignedObjectGrant,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
    ) -> Result<Option<SignedObjectGrant>, PortError>;

    async fn revoke_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn verified_domain(
        &self,
        domain: &str,
    ) -> Result<Option<VerifiedCustomDomain>, PortError>;

    async fn authoritative_inventory(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<LifecycleInventory>, PortError>;

    async fn deletion_guard(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionGuardSnapshot, PortError>;

    async fn create_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<DeletionWorkflow>, PortError>;

    /// Implementations must atomically re-read hold/retention authority and
    /// compare both the workflow and guard revisions before writing.
    async fn save_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        evidence: Option<DeletionEvidenceReceipt>,
        expected_workflow_revision: u64,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn quota_snapshot(
        &self,
        context: StorageGovernanceContextV1,
    ) -> Result<StorageQuotaSnapshot, PortError>;

    /// Applies the reservation only when the persisted quota revision equals
    /// `reservation.expected_quota_revision()`.
    async fn reserve_quota(
        &self,
        context: StorageGovernanceContextV1,
        reservation: StorageQuotaReservation,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn release_quota_reservation(
        &self,
        context: StorageGovernanceContextV1,
        reservation_id: CorrelationId,
        committed: bool,
        completed_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    /// Atomically persists the new object visibility/generation and the
    /// pending cache operation only when the current object digest matches.
    async fn begin_privacy_transition(
        &self,
        context: StorageGovernanceContextV1,
        object: &GovernedObject,
        plan: CacheInvalidationPlan,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn complete_cache_plan(
        &self,
        context: StorageGovernanceContextV1,
        receipt: CachePurgeReceipt,
    ) -> Result<StorageCasOutcomeV1, PortError>;

    async fn audit_head(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<DurableGovernanceAuditRecord>, PortError>;

    async fn append_audit(
        &self,
        context: StorageGovernanceContextV1,
        record: DurableGovernanceAuditRecord,
    ) -> Result<StorageCasOutcomeV1, PortError>;
}

pub trait SignedGrantSecretGeneratorV1: Send + Sync {
    fn generate(&self) -> Result<SignedGrantSecret, PortError>;
}

#[async_trait]
pub trait StorageGovernanceProviderV1: Send + Sync {
    /// The provider operation must be idempotent for the workflow correlation
    /// ID and target digest. A receipt is returned only after the requested
    /// target set has been observed in the required post-state.
    async fn execute_deletion_stage(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
        stage: DeletionStage,
        target_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionEvidenceReceipt, PortError>;

    /// A restore may reactivate metadata only after every manifest-bound
    /// provider object has been independently observed with its exact size and
    /// checksum. This closes the crash window between provider deletion and
    /// workflow evidence persistence.
    async fn verify_restore_possible(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
    ) -> Result<(), PortError>;

    /// Purges and then probes both successful and negative variants. Returning
    /// a receipt before both observations are absent is an adapter contract
    /// violation.
    async fn purge_and_probe_cache(
        &self,
        context: StorageGovernanceContextV1,
        plan: &CacheInvalidationPlan,
        now: TimestampMillis,
    ) -> Result<CachePurgeReceipt, PortError>;
}
