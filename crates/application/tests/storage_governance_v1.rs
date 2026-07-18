use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use frame_application::{
    StorageGovernanceService, StorageGovernanceServiceError, StorageGrantKeyMaterialV1,
    StorageGrantKeyRingV1,
};
use frame_domain::{
    ByteSize, CacheInvalidationPlan, CachePurgeReceipt, ChecksumSha256, CorrelationId,
    CustomDomainName, DeletionEvidenceReceipt, DeletionGuardSnapshot, DeletionStage,
    DeletionWorkflow, DurableGovernanceAuditRecord, GovernedObject, GovernedObjectId,
    GovernedObjectRole, GovernedObjectState, LifecycleInventory, LifecycleObject,
    MalwareDisposition, ObjectVisibility, SignedGrantId, SignedGrantKeyVersion, SignedGrantSecret,
    SignedObjectGrant, StorageAccessRequest, StorageAccessSurface, StorageActor,
    StorageAuthorizationDecision, StorageAuthorizationPolicy, StorageDenialReason,
    StorageMemberRole, StorageOperation, StorageQuotaPolicy, StorageQuotaReservation,
    StorageQuotaSnapshot, TenantId, TimestampMillis, UserId, VerifiedCustomDomain,
};
use frame_ports::{
    PortError, SignedGrantSecretGeneratorV1, StorageCasOutcomeV1, StorageGovernanceContextV1,
    StorageGovernanceProviderV1, StorageGovernanceRepositoryV1,
};

fn timestamp(value: i64) -> TimestampMillis {
    TimestampMillis::new(value).expect("timestamp")
}

fn checksum(value: u8) -> ChecksumSha256 {
    ChecksumSha256::parse(format!("{value:064x}")).expect("checksum")
}

fn zero_checksum() -> ChecksumSha256 {
    ChecksumSha256::parse("0".repeat(64)).expect("zero checksum")
}

fn object(tenant_id: TenantId, visibility: ObjectVisibility) -> GovernedObject {
    GovernedObject::new(
        tenant_id,
        GovernedObjectId::parse(format!("tenants/{tenant_id}/videos/video-one/source-r1"))
            .expect("object id"),
        GovernedObjectRole::Source,
        visibility,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        1,
        1,
        checksum(1),
        ByteSize::new(1_024).expect("size"),
        None,
    )
    .expect("object")
}

fn owner(tenant_id: TenantId) -> StorageActor {
    StorageActor::Member {
        tenant_id,
        user_id: UserId::new(),
        role: StorageMemberRole::Owner,
    }
}

fn context(tenant_id: TenantId, correlation_id: CorrelationId) -> StorageGovernanceContextV1 {
    StorageGovernanceContextV1::new(tenant_id, correlation_id, checksum(60))
}

fn complete_inventory(
    tenant_id: TenantId,
    subject: ChecksumSha256,
    authority: &GovernedObject,
) -> LifecycleInventory {
    let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
    let objects = GovernedObjectRole::ALL
        .into_iter()
        .enumerate()
        .map(|(index, role)| {
            if role == authority.role() {
                LifecycleObject {
                    tenant_id,
                    object_id: authority.object_id().clone(),
                    role,
                    checksum: authority.checksum().clone(),
                    size: authority.size(),
                    retention_until: authority.retention_until(),
                }
            } else {
                LifecycleObject {
                    tenant_id,
                    object_id: GovernedObjectId::parse(format!(
                        "tenants/{tenant_id}/objects/object-{index}"
                    ))
                    .expect("object id"),
                    role,
                    checksum: checksum(u8::try_from(index + 2).expect("small index")),
                    size: ByteSize::new(100).expect("size"),
                    retention_until: None,
                }
            }
        })
        .collect();
    LifecycleInventory::new(tenant_id, subject, &required, objects).expect("inventory")
}

#[derive(Debug)]
struct FixedSecretGenerator;

impl SignedGrantSecretGeneratorV1 for FixedSecretGenerator {
    fn generate(&self) -> Result<SignedGrantSecret, PortError> {
        SignedGrantSecret::parse(vec![0x42; 32])
            .map_err(|_| PortError::Adapter("secret generation failed".into()))
    }
}

#[derive(Debug)]
struct MemoryState {
    governed_object: GovernedObject,
    grants: BTreeMap<SignedGrantId, SignedObjectGrant>,
    domains: BTreeMap<String, VerifiedCustomDomain>,
    inventory: LifecycleInventory,
    active_hold: bool,
    guard_revision: u64,
    workflow: Option<DeletionWorkflow>,
    quota_used_bytes: ByteSize,
    quota_reserved_bytes: ByteSize,
    quota_used_objects: u64,
    quota_reserved_objects: u64,
    quota_revision: u64,
    reservations: Vec<StorageQuotaReservation>,
    cache_plan: Option<CacheInvalidationPlan>,
    cache_receipt: Option<CachePurgeReceipt>,
    audits: Vec<DurableGovernanceAuditRecord>,
}

#[derive(Debug)]
struct MemoryRepository {
    tenant_id: TenantId,
    state: Mutex<MemoryState>,
}

impl MemoryRepository {
    fn new(governed_object: GovernedObject, inventory: LifecycleInventory) -> Self {
        Self {
            tenant_id: governed_object.tenant_id(),
            state: Mutex::new(MemoryState {
                governed_object,
                grants: BTreeMap::new(),
                domains: BTreeMap::new(),
                inventory,
                active_hold: false,
                guard_revision: 1,
                workflow: None,
                quota_used_bytes: ByteSize::new(0).expect("size"),
                quota_reserved_bytes: ByteSize::new(0).expect("size"),
                quota_used_objects: 0,
                quota_reserved_objects: 0,
                quota_revision: 1,
                reservations: Vec::new(),
                cache_plan: None,
                cache_receipt: None,
                audits: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>, PortError> {
        self.state
            .lock()
            .map_err(|_| PortError::Adapter("poisoned storage test repository".into()))
    }

    fn tenant_context(&self, context: &StorageGovernanceContextV1) -> Result<(), PortError> {
        if context.tenant_id() == self.tenant_id {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }

    fn insert_domain(&self, domain: VerifiedCustomDomain) {
        self.state
            .lock()
            .expect("repository lock")
            .domains
            .insert(domain.domain().as_str().to_owned(), domain);
    }

    fn activate_hold(&self) {
        let mut state = self.state.lock().expect("repository lock");
        state.active_hold = true;
        state.guard_revision += 1;
    }

    fn audit_records(&self) -> Vec<DurableGovernanceAuditRecord> {
        self.state.lock().expect("repository lock").audits.clone()
    }

    fn pending_cache_plan(&self) -> bool {
        self.state
            .lock()
            .expect("repository lock")
            .cache_plan
            .is_some()
    }
}

#[async_trait]
impl StorageGovernanceRepositoryV1 for MemoryRepository {
    async fn governed_object(
        &self,
        context: StorageGovernanceContextV1,
        object_id: &GovernedObjectId,
    ) -> Result<Option<GovernedObject>, PortError> {
        self.tenant_context(&context)?;
        let state = self.lock()?;
        Ok((state.governed_object.object_id() == object_id).then(|| state.governed_object.clone()))
    }

    async fn insert_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant: SignedObjectGrant,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        if grant.tenant_id() != self.tenant_id {
            return Err(PortError::NotFound);
        }
        let mut state = self.lock()?;
        if state.grants.contains_key(&grant.grant_id()) {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        state.grants.insert(grant.grant_id(), grant);
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
    ) -> Result<Option<SignedObjectGrant>, PortError> {
        self.tenant_context(&context)?;
        Ok(self.lock()?.grants.get(&grant_id).cloned())
    }

    async fn revoke_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        let Some(grant) = state.grants.get_mut(&grant_id) else {
            return Err(PortError::NotFound);
        };
        if grant.revoked_at() == Some(revoked_at) {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        grant.revoke(revoked_at).map_err(|_| PortError::Conflict)?;
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn verified_domain(
        &self,
        domain: &str,
    ) -> Result<Option<VerifiedCustomDomain>, PortError> {
        Ok(self.lock()?.domains.get(domain).cloned())
    }

    async fn authoritative_inventory(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<LifecycleInventory>, PortError> {
        self.tenant_context(&context)?;
        let state = self.lock()?;
        Ok((state.inventory.subject_digest() == subject_digest).then(|| state.inventory.clone()))
    }

    async fn deletion_guard(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionGuardSnapshot, PortError> {
        self.tenant_context(&context)?;
        let state = self.lock()?;
        if state.inventory.subject_digest() != subject_digest {
            return Err(PortError::NotFound);
        }
        DeletionGuardSnapshot::new(
            self.tenant_id,
            subject_digest.clone(),
            now,
            state.guard_revision,
            state.active_hold,
            None,
        )
        .map_err(|_| PortError::Adapter("invalid deletion guard".into()))
    }

    async fn create_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        if workflow.tenant_id() != self.tenant_id
            || expected_guard_revision != state.guard_revision
            || state.active_hold
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        match &state.workflow {
            Some(existing) if existing == &workflow => Ok(StorageCasOutcomeV1::Replay),
            Some(_) => Ok(StorageCasOutcomeV1::Conflict),
            None => {
                state.workflow = Some(workflow);
                Ok(StorageCasOutcomeV1::Applied)
            }
        }
    }

    async fn deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<DeletionWorkflow>, PortError> {
        self.tenant_context(&context)?;
        let state = self.lock()?;
        Ok(state
            .workflow
            .as_ref()
            .filter(|workflow| workflow.subject_digest() == subject_digest)
            .cloned())
    }

    async fn save_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        evidence: Option<DeletionEvidenceReceipt>,
        expected_workflow_revision: u64,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        if expected_guard_revision != state.guard_revision
            || (state.active_hold && workflow.stage() != DeletionStage::Restored)
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        let Some(existing) = state.workflow.as_ref() else {
            return Ok(StorageCasOutcomeV1::Conflict);
        };
        if existing == &workflow {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        if existing.revision() != expected_workflow_revision
            || workflow.revision() != expected_workflow_revision + 1
            || workflow.inventory_digest() != state.inventory.digest()
            || (workflow.stage() != DeletionStage::Complete
                && workflow.stage() != DeletionStage::Restored
                && evidence.as_ref().map(DeletionEvidenceReceipt::stage) != Some(workflow.stage()))
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        state.workflow = Some(workflow);
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn quota_snapshot(
        &self,
        context: StorageGovernanceContextV1,
    ) -> Result<StorageQuotaSnapshot, PortError> {
        self.tenant_context(&context)?;
        let state = self.lock()?;
        StorageQuotaSnapshot::new(
            self.tenant_id,
            state.quota_used_bytes,
            state.quota_reserved_bytes,
            state.quota_used_objects,
            state.quota_reserved_objects,
            state.quota_revision,
        )
        .map_err(|_| PortError::Adapter("invalid quota state".into()))
    }

    async fn reserve_quota(
        &self,
        context: StorageGovernanceContextV1,
        reservation: StorageQuotaReservation,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        if reservation.tenant_id() != self.tenant_id
            || reservation.expected_quota_revision() != state.quota_revision
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        if state
            .reservations
            .iter()
            .any(|stored| stored.reservation_id() == reservation.reservation_id())
        {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        state.quota_reserved_bytes = state
            .quota_reserved_bytes
            .checked_add(reservation.requested_bytes())
            .map_err(|_| PortError::Conflict)?;
        state.quota_reserved_objects += 1;
        state.quota_revision += 1;
        state.reservations.push(reservation);
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn release_quota_reservation(
        &self,
        context: StorageGovernanceContextV1,
        reservation_id: CorrelationId,
        committed: bool,
        _completed_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        let Some(index) = state
            .reservations
            .iter()
            .position(|reservation| reservation.reservation_id() == reservation_id)
        else {
            return Ok(StorageCasOutcomeV1::Replay);
        };
        let reservation = state.reservations.remove(index);
        state.quota_reserved_bytes =
            ByteSize::new(state.quota_reserved_bytes.get() - reservation.requested_bytes().get())
                .map_err(|_| PortError::Adapter("invalid quota release".into()))?;
        state.quota_reserved_objects -= 1;
        if committed {
            state.quota_used_bytes = state
                .quota_used_bytes
                .checked_add(reservation.requested_bytes())
                .map_err(|_| PortError::Conflict)?;
            state.quota_used_objects += 1;
        }
        state.quota_revision += 1;
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn begin_privacy_transition(
        &self,
        context: StorageGovernanceContextV1,
        governed_object: &GovernedObject,
        plan: CacheInvalidationPlan,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        if governed_object != &state.governed_object
            || plan.tenant_id() != self.tenant_id
            || plan.object_id() != governed_object.object_id()
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        match &state.cache_plan {
            Some(existing) if existing == &plan => Ok(StorageCasOutcomeV1::Replay),
            Some(_) => Ok(StorageCasOutcomeV1::Conflict),
            None => {
                state.cache_plan = Some(plan);
                Ok(StorageCasOutcomeV1::Applied)
            }
        }
    }

    async fn complete_cache_plan(
        &self,
        context: StorageGovernanceContextV1,
        receipt: CachePurgeReceipt,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        let Some(plan) = state.cache_plan.as_ref() else {
            return Ok(StorageCasOutcomeV1::Conflict);
        };
        plan.verify_receipt(&receipt)
            .map_err(|_| PortError::Conflict)?;
        if state.cache_receipt.as_ref() == Some(&receipt) {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        state.cache_receipt = Some(receipt);
        Ok(StorageCasOutcomeV1::Applied)
    }

    async fn audit_head(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<DurableGovernanceAuditRecord>, PortError> {
        if tenant_id != self.tenant_id {
            return Err(PortError::NotFound);
        }
        Ok(self.lock()?.audits.last().cloned())
    }

    async fn append_audit(
        &self,
        context: StorageGovernanceContextV1,
        record: DurableGovernanceAuditRecord,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        self.tenant_context(&context)?;
        let mut state = self.lock()?;
        if !record.verify() || record.tenant_id() != self.tenant_id {
            return Err(PortError::InvalidRequest("invalid audit record".into()));
        }
        if state
            .audits
            .iter()
            .any(|stored| stored.digest() == record.digest())
        {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        let expected_sequence = u64::try_from(state.audits.len()).unwrap_or(u64::MAX) + 1;
        let expected_previous = state
            .audits
            .last()
            .map_or_else(zero_checksum, |stored| stored.digest().clone());
        if record.sequence() != expected_sequence
            || record.previous_digest() != &expected_previous
            || state
                .audits
                .last()
                .is_some_and(|stored| record.occurred_at() < stored.occurred_at())
        {
            return Ok(StorageCasOutcomeV1::Conflict);
        }
        state.audits.push(record);
        Ok(StorageCasOutcomeV1::Applied)
    }
}

#[derive(Debug)]
struct TestProvider {
    negative_cache_absent: bool,
    restore_possible: bool,
    hold_race: Option<Arc<MemoryRepository>>,
    deletion_calls: AtomicUsize,
}

impl TestProvider {
    fn honest() -> Self {
        Self {
            negative_cache_absent: true,
            restore_possible: true,
            hold_race: None,
            deletion_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl StorageGovernanceProviderV1 for TestProvider {
    async fn execute_deletion_stage(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
        stage: DeletionStage,
        target_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionEvidenceReceipt, PortError> {
        self.deletion_calls.fetch_add(1, Ordering::SeqCst);
        let cache_stage = stage == DeletionStage::CachePurged;
        let receipt = DeletionEvidenceReceipt::verified(
            context.tenant_id(),
            inventory.digest().clone(),
            stage,
            target_digest.clone(),
            checksum(u8::try_from(workflow.revision() + 80).unwrap_or(99)),
            now,
            cache_stage,
            cache_stage,
        )
        .map_err(|_| PortError::Adapter("invalid provider receipt".into()))?;
        if let Some(repository) = &self.hold_race {
            repository.activate_hold();
        }
        Ok(receipt)
    }

    async fn verify_restore_possible(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
    ) -> Result<(), PortError> {
        if !self.restore_possible {
            return Err(PortError::Conflict);
        }
        if context.tenant_id() == inventory.tenant_id()
            && workflow.tenant_id() == inventory.tenant_id()
            && workflow.inventory_digest() == inventory.digest()
        {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }

    async fn purge_and_probe_cache(
        &self,
        context: StorageGovernanceContextV1,
        plan: &CacheInvalidationPlan,
        now: TimestampMillis,
    ) -> Result<CachePurgeReceipt, PortError> {
        CachePurgeReceipt::verified_absence(
            context.tenant_id(),
            plan.object_id().clone(),
            plan.from_generation(),
            plan.cache_tag_digest(),
            checksum(90),
            now,
            true,
            self.negative_cache_absent,
        )
        .map_err(|_| PortError::Adapter("invalid cache receipt".into()))
    }
}

#[test]
fn authorization_penetration_matrix_denies_cross_tenant_and_direct_origin_paths() {
    let tenant = TenantId::new();
    let resource = object(tenant, ObjectVisibility::Private);
    let operations = [
        StorageOperation::Read,
        StorageOperation::ReadRange,
        StorageOperation::WriteImmutable,
        StorageOperation::List,
        StorageOperation::Copy,
        StorageOperation::Sign,
        StorageOperation::Delete,
        StorageOperation::Restore,
        StorageOperation::Export,
        StorageOperation::PurgeCache,
        StorageOperation::ManageCustomDomain,
    ];
    for operation in operations {
        for surface in [
            StorageAccessSurface::SameOriginApplication,
            StorageAccessSurface::DirectOrigin,
            StorageAccessSurface::SignedRoute,
            StorageAccessSurface::CustomDomain,
            StorageAccessSurface::MediaTransformation,
        ] {
            let result = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
                actor: owner(TenantId::new()),
                operation,
                surface,
                object: &resource,
                now: timestamp(10),
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            });
            assert!(matches!(result, StorageAuthorizationDecision::Deny(_)));
        }
        let direct = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: owner(tenant),
            operation,
            surface: StorageAccessSurface::DirectOrigin,
            object: &resource,
            now: timestamp(10),
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        });
        assert_eq!(
            direct,
            StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied)
        );
    }
    assert_eq!(
        StorageDenialReason::DomainInvalid.public_code(),
        "storage_access_denied"
    );
    assert_eq!(
        StorageDenialReason::GrantInvalid.public_code(),
        "storage_access_denied"
    );
}

#[tokio::test]
async fn persisted_hmac_grants_enforce_not_before_tenant_domain_forgery_and_revocation() {
    let tenant = TenantId::new();
    let resource = object(tenant, ObjectVisibility::Unlisted);
    let inventory = complete_inventory(tenant, checksum(50), &resource);
    let repository = MemoryRepository::new(resource.clone(), inventory);
    repository.insert_domain(
        VerifiedCustomDomain::new(
            tenant,
            CustomDomainName::parse("media.example.com").expect("domain"),
            1,
            true,
        )
        .expect("binding"),
    );
    let key_version = SignedGrantKeyVersion::new(7).expect("key version");
    let key_ring = StorageGrantKeyRingV1::new(
        key_version,
        [(
            key_version,
            StorageGrantKeyMaterialV1::parse(vec![0x11; 32]).expect("key"),
        )],
    )
    .expect("key ring");
    let service =
        StorageGovernanceService::with_signing_keys(["https://app.example".to_owned()], key_ring)
            .expect("governance service");
    let issued = service
        .issue_read_grant(
            &repository,
            &FixedSecretGenerator,
            context(tenant, CorrelationId::new()),
            owner(tenant),
            &resource,
            StorageOperation::Read,
            timestamp(10),
            timestamp(100),
        )
        .await
        .expect("grant");
    let token = issued.opaque_token();

    assert!(matches!(
        service
            .authorize_persisted_read(
                &repository,
                context(tenant, CorrelationId::new()),
                &resource,
                issued.grant_id(),
                &token,
                StorageOperation::Read,
                StorageAccessSurface::SignedRoute,
                None,
                timestamp(9),
                "video/mp4",
                None,
                false,
            )
            .await,
        Err(StorageGovernanceServiceError::Denied(_))
    ));
    let read = service
        .authorize_persisted_read(
            &repository,
            context(tenant, CorrelationId::new()),
            &resource,
            issued.grant_id(),
            &token,
            StorageOperation::Read,
            StorageAccessSurface::SignedRoute,
            None,
            timestamp(50),
            "video/mp4",
            Some("https://app.example"),
            false,
        )
        .await
        .expect("signed read");
    assert_eq!(read.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        read.headers()["cache-control"],
        "private, no-store, max-age=0"
    );

    let forged = "09".repeat(32);
    assert!(matches!(
        service
            .authorize_persisted_read(
                &repository,
                context(tenant, CorrelationId::new()),
                &resource,
                issued.grant_id(),
                &forged,
                StorageOperation::Read,
                StorageAccessSurface::SignedRoute,
                None,
                timestamp(50),
                "video/mp4",
                None,
                false,
            )
            .await,
        Err(StorageGovernanceServiceError::Denied(_))
    ));
    assert!(matches!(
        service
            .authorize_persisted_read(
                &repository,
                context(TenantId::new(), CorrelationId::new()),
                &resource,
                issued.grant_id(),
                &token,
                StorageOperation::Read,
                StorageAccessSurface::SignedRoute,
                None,
                timestamp(50),
                "video/mp4",
                None,
                false,
            )
            .await,
        Err(StorageGovernanceServiceError::Denied(_))
    ));
    service
        .authorize_persisted_read(
            &repository,
            context(tenant, CorrelationId::new()),
            &resource,
            issued.grant_id(),
            &token,
            StorageOperation::Read,
            StorageAccessSurface::CustomDomain,
            Some("media.example.com"),
            timestamp(50),
            "video/mp4",
            None,
            false,
        )
        .await
        .expect("verified custom domain read");
    assert!(matches!(
        service
            .authorize_persisted_read(
                &repository,
                context(tenant, CorrelationId::new()),
                &resource,
                issued.grant_id(),
                &token,
                StorageOperation::Read,
                StorageAccessSurface::CustomDomain,
                Some("attacker.example.com"),
                timestamp(50),
                "video/mp4",
                None,
                false,
            )
            .await,
        Err(StorageGovernanceServiceError::Denied(_))
    ));
    service
        .revoke_read_grant(
            &repository,
            context(tenant, CorrelationId::new()),
            issued.grant_id(),
            timestamp(60),
        )
        .await
        .expect("revoke");
    assert!(matches!(
        service
            .authorize_persisted_read(
                &repository,
                context(tenant, CorrelationId::new()),
                &resource,
                issued.grant_id(),
                &token,
                StorageOperation::Read,
                StorageAccessSurface::SignedRoute,
                None,
                timestamp(61),
                "video/mp4",
                None,
                false,
            )
            .await,
        Err(StorageGovernanceServiceError::Denied(_))
    ));

    let audit = repository.audit_records();
    assert_eq!(audit.len(), 4);
    assert!(audit.iter().all(DurableGovernanceAuditRecord::verify));
    assert_eq!(audit[0].previous_digest(), &zero_checksum());
    for pair in audit.windows(2) {
        assert_eq!(pair[1].previous_digest(), pair[0].digest());
    }
}

#[test]
fn hold_release_and_manifest_driven_erasure_rehearsal_is_deterministic() {
    let tenant = TenantId::new();
    let authority = object(tenant, ObjectVisibility::Private);
    let subject = checksum(63);
    let inventory = complete_inventory(tenant, subject.clone(), &authority);
    let active = DeletionGuardSnapshot::new(tenant, subject.clone(), timestamp(1), 1, true, None)
        .expect("active guard");
    assert!(matches!(
        DeletionWorkflow::plan(CorrelationId::new(), &inventory, &active, timestamp(2)),
        Err(frame_domain::StorageGovernanceError::LegalHoldActive)
    ));
    let released = DeletionGuardSnapshot::new(tenant, subject, timestamp(3), 2, false, None)
        .expect("released guard");
    let mut deletion =
        DeletionWorkflow::plan(CorrelationId::new(), &inventory, &released, timestamp(4))
            .expect("deletion");
    for (index, stage) in [
        DeletionStage::Tombstoned,
        DeletionStage::OriginDeleted,
        DeletionStage::CachePurged,
        DeletionStage::BackupDeleted,
        DeletionStage::Verified,
    ]
    .into_iter()
    .enumerate()
    {
        let cache_stage = stage == DeletionStage::CachePurged;
        let receipt = DeletionEvidenceReceipt::verified(
            tenant,
            inventory.digest().clone(),
            stage,
            deletion.evidence_target_digest(stage).expect("target"),
            checksum(u8::try_from(index + 70).expect("small index")),
            timestamp(4),
            cache_stage,
            cache_stage,
        )
        .expect("receipt");
        deletion
            .record(&inventory, &receipt, &released, timestamp(4))
            .expect("stage");
    }
    deletion
        .complete(&inventory, &released, timestamp(4))
        .expect("complete");
    let proof = deletion.completion_proof(timestamp(5)).expect("proof");
    let json = serde_json::to_string(&proof).expect("proof JSON");
    assert!(!json.contains("objects/"));
    assert!(!json.contains("video-one"));
}

#[test]
fn forged_evidence_and_manifest_swaps_cannot_complete_deletion() {
    let tenant = TenantId::new();
    let authority = object(tenant, ObjectVisibility::Private);
    let inventory = complete_inventory(tenant, checksum(50), &authority);
    let other = complete_inventory(tenant, checksum(51), &authority);
    let guard = DeletionGuardSnapshot::new(tenant, checksum(50), timestamp(1), 1, false, None)
        .expect("guard");
    let mut deletion =
        DeletionWorkflow::plan(CorrelationId::new(), &inventory, &guard, timestamp(1))
            .expect("deletion");
    let forged = DeletionEvidenceReceipt::verified(
        tenant,
        inventory.digest().clone(),
        DeletionStage::Tombstoned,
        checksum(52),
        checksum(53),
        timestamp(1),
        false,
        false,
    )
    .expect("forged shape");
    assert!(
        deletion
            .record(&inventory, &forged, &guard, timestamp(1))
            .is_err()
    );
    let valid = DeletionEvidenceReceipt::verified(
        tenant,
        inventory.digest().clone(),
        DeletionStage::Tombstoned,
        deletion
            .evidence_target_digest(DeletionStage::Tombstoned)
            .expect("target"),
        checksum(54),
        timestamp(1),
        false,
        false,
    )
    .expect("receipt");
    assert!(
        deletion
            .record(&other, &valid, &guard, timestamp(1))
            .is_err()
    );
}

#[tokio::test]
async fn quota_reservations_include_outstanding_usage_and_hold_races_fail_closed() {
    let tenant = TenantId::new();
    let authority = object(tenant, ObjectVisibility::Private);
    let subject = checksum(50);
    let inventory = complete_inventory(tenant, subject.clone(), &authority);
    let repository = Arc::new(MemoryRepository::new(authority.clone(), inventory));
    let service = StorageGovernanceService::new(Vec::new()).expect("service");
    let policy = StorageQuotaPolicy::new(ByteSize::new(100).expect("size"), 2).expect("quota");
    let first = service.reserve_quota(
        repository.as_ref(),
        context(tenant, CorrelationId::new()),
        policy,
        ByteSize::new(60).expect("size"),
        timestamp(1),
        timestamp(100),
    );
    let second = service.reserve_quota(
        repository.as_ref(),
        context(tenant, CorrelationId::new()),
        policy,
        ByteSize::new(60).expect("size"),
        timestamp(1),
        timestamp(100),
    );
    let (first, second) = tokio::join!(first, second);
    assert_ne!(first.is_ok(), second.is_ok());
    assert!(matches!(
        first.err().or_else(|| second.err()),
        Some(StorageGovernanceServiceError::Contract(
            frame_domain::StorageGovernanceError::QuotaExceeded
        ))
    ));

    service
        .start_deletion(
            repository.as_ref(),
            context(tenant, CorrelationId::new()),
            owner(tenant),
            &authority,
            &subject,
            timestamp(2),
        )
        .await
        .expect("workflow");
    let racing_provider = TestProvider {
        negative_cache_absent: true,
        restore_possible: true,
        hold_race: Some(Arc::clone(&repository)),
        deletion_calls: AtomicUsize::new(0),
    };
    assert_eq!(
        service
            .advance_deletion(
                repository.as_ref(),
                &racing_provider,
                context(tenant, CorrelationId::new()),
                owner(tenant),
                &authority,
                &subject,
                DeletionStage::Tombstoned,
                timestamp(3),
            )
            .await,
        Err(StorageGovernanceServiceError::StateConflict)
    );
    assert_eq!(racing_provider.deletion_calls.load(Ordering::SeqCst), 1);
    let persisted = repository
        .deletion_workflow(context(tenant, CorrelationId::new()), &subject)
        .await
        .expect("repository")
        .expect("workflow");
    assert_eq!(persisted.stage(), DeletionStage::Planned);
}

#[tokio::test]
async fn restore_never_reactivates_metadata_after_provider_bytes_disappear() {
    let tenant = TenantId::new();
    let authority = object(tenant, ObjectVisibility::Private);
    let subject = checksum(51);
    let inventory = complete_inventory(tenant, subject.clone(), &authority);
    let repository = MemoryRepository::new(authority.clone(), inventory);
    let service = StorageGovernanceService::new(Vec::new()).expect("service");
    service
        .start_deletion(
            &repository,
            context(tenant, CorrelationId::new()),
            owner(tenant),
            &authority,
            &subject,
            timestamp(2),
        )
        .await
        .expect("workflow");
    let missing_provider_bytes = TestProvider {
        negative_cache_absent: true,
        restore_possible: false,
        hold_race: None,
        deletion_calls: AtomicUsize::new(0),
    };
    assert_eq!(
        service
            .restore_deletion(
                &repository,
                &missing_provider_bytes,
                context(tenant, CorrelationId::new()),
                owner(tenant),
                &authority,
                &subject,
                timestamp(3),
            )
            .await,
        Err(StorageGovernanceServiceError::StateConflict)
    );
    let persisted = repository
        .deletion_workflow(context(tenant, CorrelationId::new()), &subject)
        .await
        .expect("repository")
        .expect("workflow");
    assert_eq!(persisted.stage(), DeletionStage::Planned);
    let restored = service
        .restore_deletion(
            &repository,
            &TestProvider::honest(),
            context(tenant, CorrelationId::new()),
            owner(tenant),
            &authority,
            &subject,
            timestamp(4),
        )
        .await
        .expect("restore");
    assert_eq!(restored.stage(), DeletionStage::Restored);
}

#[tokio::test]
async fn privacy_transition_requires_positive_and_negative_absence_inside_the_slo() {
    let tenant = TenantId::new();
    let resource = object(tenant, ObjectVisibility::Public);
    let inventory = complete_inventory(tenant, checksum(50), &resource);
    let repository = MemoryRepository::new(resource.clone(), inventory);
    let service = StorageGovernanceService::new(Vec::new()).expect("service");
    let forged_provider = TestProvider {
        negative_cache_absent: false,
        restore_possible: true,
        hold_race: None,
        deletion_calls: AtomicUsize::new(0),
    };
    assert!(matches!(
        service
            .execute_privacy_change(
                &repository,
                &forged_provider,
                context(tenant, CorrelationId::new()),
                owner(tenant),
                &resource,
                ObjectVisibility::Private,
                2,
                timestamp(10),
                1_000,
                timestamp(11),
            )
            .await,
        Err(StorageGovernanceServiceError::Contract(
            frame_domain::StorageGovernanceError::CachePurgeUnverified
        ))
    ));
    assert!(repository.pending_cache_plan());

    let honest_repository = MemoryRepository::new(
        resource.clone(),
        complete_inventory(tenant, checksum(51), &resource),
    );
    service
        .execute_privacy_change(
            &honest_repository,
            &TestProvider::honest(),
            context(tenant, CorrelationId::new()),
            owner(tenant),
            &resource,
            ObjectVisibility::Private,
            2,
            timestamp(10),
            1_000,
            timestamp(11),
        )
        .await
        .expect("verified purge");
}
