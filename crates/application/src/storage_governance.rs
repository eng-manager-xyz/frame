use std::collections::{BTreeMap, BTreeSet};

use frame_domain::{
    ByteSize, CacheInvalidationPlan, CachePurgeReceipt, ChecksumSha256, CorrelationId,
    CorsOriginV1, CustomDomainName, DeletionGuardSnapshot, DeletionStage, DeletionWorkflow,
    DurableGovernanceAuditRecord, GovernedObject, LifecycleInventory, SignedGrantId,
    SignedGrantKeyVersion, SignedGrantSecret, SignedObjectGrant, StorageAccessRequest,
    StorageAccessSurface, StorageActor, StorageAuthorizationDecision, StorageAuthorizationPolicy,
    StorageDenialReason, StorageExportPlan, StorageGovernanceError, StorageOperation,
    StorageQuotaPolicy, StorageQuotaReservation, StorageResponsePolicy, TimestampMillis,
};
use frame_ports::{
    PortError, SignedGrantSecretGeneratorV1, StorageCasOutcomeV1, StorageGovernanceContextV1,
    StorageGovernanceProviderV1, StorageGovernanceRepositoryV1,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, PartialEq, Eq)]
pub struct StorageGrantKeyMaterialV1(Vec<u8>);

impl StorageGrantKeyMaterialV1 {
    pub fn parse(value: impl Into<Vec<u8>>) -> Result<Self, StorageGovernanceServiceError> {
        let value = value.into();
        if !(32..=64).contains(&value.len()) {
            return Err(StorageGovernanceServiceError::InvalidConfiguration);
        }
        Ok(Self(value))
    }
}

impl std::fmt::Debug for StorageGrantKeyMaterialV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StorageGrantKeyMaterialV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageGrantKeyRingV1 {
    active: SignedGrantKeyVersion,
    keys: BTreeMap<SignedGrantKeyVersion, StorageGrantKeyMaterialV1>,
}

impl StorageGrantKeyRingV1 {
    pub fn new(
        active: SignedGrantKeyVersion,
        keys: impl IntoIterator<Item = (SignedGrantKeyVersion, StorageGrantKeyMaterialV1)>,
    ) -> Result<Self, StorageGovernanceServiceError> {
        let mut unique = BTreeMap::new();
        for (version, key) in keys {
            if unique.insert(version, key).is_some() {
                return Err(StorageGovernanceServiceError::InvalidConfiguration);
            }
        }
        if unique.is_empty() || unique.len() > 8 || !unique.contains_key(&active) {
            return Err(StorageGovernanceServiceError::InvalidConfiguration);
        }
        Ok(Self {
            active,
            keys: unique,
        })
    }

    #[must_use]
    pub const fn active_version(&self) -> SignedGrantKeyVersion {
        self.active
    }

    fn digest(
        &self,
        version: SignedGrantKeyVersion,
        secret: &SignedGrantSecret,
    ) -> Option<ChecksumSha256> {
        let key = self.keys.get(&version)?;
        let mut message = Vec::new();
        append_frame(&mut message, b"frame.storage.read-grant.v1");
        append_frame(&mut message, &version.get().to_be_bytes());
        append_frame(&mut message, secret.expose_for_hmac());
        Some(ChecksumSha256::digest_bytes(&hmac_sha256(&key.0, &message)))
    }
}

impl std::fmt::Debug for StorageGrantKeyRingV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageGrantKeyRingV1")
            .field("active", &self.active)
            .field("key_count", &self.keys.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssuedStorageReadGrantV1 {
    grant_id: SignedGrantId,
    secret: SignedGrantSecret,
    expires_at: TimestampMillis,
}

impl IssuedStorageReadGrantV1 {
    #[must_use]
    pub const fn grant_id(&self) -> SignedGrantId {
        self.grant_id
    }

    #[must_use]
    pub fn opaque_token(&self) -> String {
        self.secret.opaque_token()
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }
}

impl std::fmt::Debug for IssuedStorageReadGrantV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedStorageReadGrantV1")
            .field("grant_id", &self.grant_id)
            .field("expires_at", &self.expires_at)
            .field("secret", &"[redacted]")
            .finish()
    }
}

/// Runtime-neutral entry point for every object authorization decision.
///
/// Adapters may resolve state and perform I/O, but they cannot mint grants, construct response
/// headers, export, or start deletion without passing this service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageGovernanceService {
    allowed_origins: BTreeSet<String>,
    grant_keys: Option<StorageGrantKeyRingV1>,
}

impl StorageGovernanceService {
    pub fn new(
        allowed_origins: impl IntoIterator<Item = String>,
    ) -> Result<Self, StorageGovernanceServiceError> {
        let allowed_origins = allowed_origins.into_iter().collect::<BTreeSet<_>>();
        if allowed_origins.len() > 64
            || allowed_origins
                .iter()
                .any(|origin| !is_canonical_configured_origin(origin))
        {
            return Err(StorageGovernanceServiceError::InvalidConfiguration);
        }
        Ok(Self {
            allowed_origins,
            grant_keys: None,
        })
    }

    pub fn with_signing_keys(
        allowed_origins: impl IntoIterator<Item = String>,
        grant_keys: StorageGrantKeyRingV1,
    ) -> Result<Self, StorageGovernanceServiceError> {
        let mut service = Self::new(allowed_origins)?;
        service.grant_keys = Some(grant_keys);
        Ok(service)
    }

    pub fn authorize(
        &self,
        correlation_id: CorrelationId,
        request: StorageAccessRequest<'_>,
    ) -> Result<StorageAuthorizationReceipt, StorageGovernanceServiceError> {
        let operation = request.operation;
        let surface = request.surface;
        let resource_digest = request.object.audit_digest();
        match StorageAuthorizationPolicy::evaluate(request) {
            StorageAuthorizationDecision::Allow => Ok(StorageAuthorizationReceipt {
                correlation_id,
                operation,
                surface,
                resource_digest,
            }),
            StorageAuthorizationDecision::Deny(reason) => {
                Err(StorageGovernanceServiceError::Denied(reason))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn authorize_read(
        &self,
        correlation_id: CorrelationId,
        request: StorageAccessRequest<'_>,
        content_type: &str,
        request_origin: Option<&str>,
        download: bool,
    ) -> Result<AuthorizedObjectRead, StorageGovernanceServiceError> {
        if !matches!(
            request.operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ) {
            return Err(StorageGovernanceServiceError::InvalidRequest);
        }
        let visibility = request.object.visibility();
        let receipt = self.authorize(correlation_id, request)?;
        let policy = StorageResponsePolicy::for_object(
            content_type,
            visibility,
            request_origin,
            &self.allowed_origins,
            download,
        )?;
        Ok(AuthorizedObjectRead { receipt, policy })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn issue_read_grant(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        secret_generator: &dyn SignedGrantSecretGeneratorV1,
        context: StorageGovernanceContextV1,
        actor: StorageActor,
        object: &GovernedObject,
        operation: StorageOperation,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
    ) -> Result<IssuedStorageReadGrantV1, StorageGovernanceServiceError> {
        if !matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ) {
            return Err(StorageGovernanceServiceError::InvalidRequest);
        }
        self.authorize(
            context.correlation_id(),
            StorageAccessRequest {
                actor,
                operation: StorageOperation::Sign,
                surface: StorageAccessSurface::SameOriginApplication,
                object,
                now: issued_at,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        let keys = self
            .grant_keys
            .as_ref()
            .ok_or(StorageGovernanceServiceError::SigningUnavailable)?;
        let secret = secret_generator.generate().map_err(map_port_error)?;
        let digest = keys
            .digest(keys.active_version(), &secret)
            .ok_or(StorageGovernanceServiceError::SigningUnavailable)?;
        let grant_id = SignedGrantId::new();
        let grant = SignedObjectGrant::persisted(
            grant_id,
            keys.active_version(),
            object,
            operation,
            issued_at,
            expires_at,
            digest,
        )?;
        match repository
            .insert_signed_grant(context.clone(), grant)
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied => {}
            StorageCasOutcomeV1::Replay | StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            issued_at,
            "grant_issue",
            "allowed",
            object.audit_digest(),
        )
        .await?;
        Ok(IssuedStorageReadGrantV1 {
            grant_id,
            secret,
            expires_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn authorize_persisted_read(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        object: &GovernedObject,
        grant_id: SignedGrantId,
        opaque_secret: &str,
        operation: StorageOperation,
        surface: StorageAccessSurface,
        request_domain_name: Option<&str>,
        now: TimestampMillis,
        content_type: &str,
        request_origin: Option<&str>,
        download: bool,
    ) -> Result<AuthorizedObjectRead, StorageGovernanceServiceError> {
        if !matches!(
            surface,
            StorageAccessSurface::SignedRoute | StorageAccessSurface::CustomDomain
        ) || !matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ) {
            return Err(StorageGovernanceServiceError::InvalidRequest);
        }
        let grant = repository
            .signed_grant(context.clone(), grant_id)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::Denied(
                StorageDenialReason::GrantInvalid,
            ))?;
        let secret = SignedGrantSecret::parse_opaque_token(opaque_secret)?;
        let keys = self
            .grant_keys
            .as_ref()
            .ok_or(StorageGovernanceServiceError::SigningUnavailable)?;
        let digest = keys.digest(grant.key_version(), &secret).ok_or(
            StorageGovernanceServiceError::Denied(StorageDenialReason::GrantInvalid),
        )?;
        let request_domain = if surface == StorageAccessSurface::CustomDomain {
            Some(CustomDomainName::parse(request_domain_name.ok_or(
                StorageGovernanceServiceError::Denied(StorageDenialReason::DomainInvalid),
            )?)?)
        } else {
            if request_domain_name.is_some() {
                return Err(StorageGovernanceServiceError::InvalidRequest);
            }
            None
        };
        let custom_domain = if let Some(domain) = request_domain.as_ref() {
            repository
                .verified_domain(domain.as_str())
                .await
                .map_err(map_port_error)?
        } else {
            None
        };
        let read = self.authorize_read(
            context.correlation_id(),
            StorageAccessRequest {
                actor: StorageActor::Anonymous,
                operation,
                surface,
                object,
                now,
                grant: Some(&grant),
                grant_proof: Some(&digest),
                request_domain: request_domain.as_ref(),
                custom_domain: custom_domain.as_ref(),
            },
            content_type,
            request_origin,
            download,
        )?;
        self.append_audit(
            repository,
            context,
            now,
            "object_read",
            "allowed",
            object.audit_digest(),
        )
        .await?;
        Ok(read)
    }

    pub async fn revoke_read_grant(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<(), StorageGovernanceServiceError> {
        let grant = repository
            .signed_grant(context.clone(), grant_id)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::Denied(
                StorageDenialReason::GrantInvalid,
            ))?;
        let object = repository
            .governed_object(context.clone(), grant.object_id())
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::Denied(
                StorageDenialReason::GrantInvalid,
            ))?;
        match repository
            .revoke_signed_grant(context.clone(), grant_id, revoked_at)
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => Ok(()),
            StorageCasOutcomeV1::Conflict => Err(StorageGovernanceServiceError::StateConflict),
        }?;
        self.append_audit(
            repository,
            context,
            revoked_at,
            "grant_revoke",
            "allowed",
            object.audit_digest(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plan_privacy_change(
        &self,
        correlation_id: CorrelationId,
        actor: StorageActor,
        object: &GovernedObject,
        to_visibility: frame_domain::ObjectVisibility,
        to_generation: u64,
        changed_at: TimestampMillis,
        cache_slo_ms: i64,
    ) -> Result<CacheInvalidationPlan, StorageGovernanceServiceError> {
        self.authorize(
            correlation_id,
            StorageAccessRequest {
                actor,
                operation: StorageOperation::PurgeCache,
                surface: StorageAccessSurface::SameOriginApplication,
                object,
                now: changed_at,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        CacheInvalidationPlan::privacy_change(
            object,
            to_visibility,
            to_generation,
            changed_at,
            cache_slo_ms,
        )
        .map_err(Into::into)
    }

    pub fn plan_deletion(
        &self,
        correlation_id: CorrelationId,
        actor: StorageActor,
        authority_object: &GovernedObject,
        inventory: &LifecycleInventory,
        guard: &DeletionGuardSnapshot,
        requested_at: TimestampMillis,
    ) -> Result<DeletionWorkflow, StorageGovernanceServiceError> {
        self.authorize(
            correlation_id,
            StorageAccessRequest {
                actor,
                operation: StorageOperation::Delete,
                surface: StorageAccessSurface::SameOriginApplication,
                object: authority_object,
                now: requested_at,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        if inventory.tenant_id() != authority_object.tenant_id()
            || !inventory.contains_governed_object(authority_object)
        {
            return Err(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied,
            ));
        }
        DeletionWorkflow::plan(correlation_id, inventory, guard, requested_at).map_err(Into::into)
    }

    pub fn plan_export(
        &self,
        correlation_id: CorrelationId,
        actor: StorageActor,
        authority_object: &GovernedObject,
        inventory: &LifecycleInventory,
        now: TimestampMillis,
    ) -> Result<StorageExportPlan, StorageGovernanceServiceError> {
        self.authorize(
            correlation_id,
            StorageAccessRequest {
                actor,
                operation: StorageOperation::Export,
                surface: StorageAccessSurface::SameOriginApplication,
                object: authority_object,
                now,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        if inventory.tenant_id() != authority_object.tenant_id()
            || !inventory.contains_governed_object(authority_object)
        {
            return Err(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied,
            ));
        }
        Ok(StorageExportPlan::from_manifest(inventory))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_privacy_change(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        provider: &dyn StorageGovernanceProviderV1,
        context: StorageGovernanceContextV1,
        actor: StorageActor,
        object: &GovernedObject,
        to_visibility: frame_domain::ObjectVisibility,
        to_generation: u64,
        changed_at: TimestampMillis,
        cache_slo_ms: i64,
        probe_at: TimestampMillis,
    ) -> Result<CachePurgeReceipt, StorageGovernanceServiceError> {
        let plan = self.plan_privacy_change(
            context.correlation_id(),
            actor,
            object,
            to_visibility,
            to_generation,
            changed_at,
            cache_slo_ms,
        )?;
        match repository
            .begin_privacy_transition(context.clone(), object, plan.clone())
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        let receipt = provider
            .purge_and_probe_cache(context.clone(), &plan, probe_at)
            .await
            .map_err(map_port_error)?;
        plan.verify_receipt(&receipt)?;
        match repository
            .complete_cache_plan(context.clone(), receipt.clone())
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            probe_at,
            "privacy_change",
            "purged",
            object.audit_digest(),
        )
        .await?;
        Ok(receipt)
    }

    pub async fn start_deletion(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        actor: StorageActor,
        authority_object: &GovernedObject,
        subject_digest: &ChecksumSha256,
        requested_at: TimestampMillis,
    ) -> Result<DeletionWorkflow, StorageGovernanceServiceError> {
        let inventory = repository
            .authoritative_inventory(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied,
            ))?;
        let guard = repository
            .deletion_guard(context.clone(), subject_digest, requested_at)
            .await
            .map_err(map_port_error)?;
        let workflow = self.plan_deletion(
            context.correlation_id(),
            actor,
            authority_object,
            &inventory,
            &guard,
            requested_at,
        )?;
        match repository
            .create_deletion_workflow(
                context.clone(),
                workflow.clone(),
                guard.authority_revision(),
            )
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            requested_at,
            "deletion_start",
            "planned",
            authority_object.audit_digest(),
        )
        .await?;
        Ok(workflow)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn advance_deletion(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        provider: &dyn StorageGovernanceProviderV1,
        context: StorageGovernanceContextV1,
        actor: StorageActor,
        authority_object: &GovernedObject,
        subject_digest: &ChecksumSha256,
        next: DeletionStage,
        now: TimestampMillis,
    ) -> Result<DeletionWorkflow, StorageGovernanceServiceError> {
        self.authorize(
            context.correlation_id(),
            StorageAccessRequest {
                actor,
                operation: if next == DeletionStage::CachePurged {
                    StorageOperation::PurgeCache
                } else {
                    StorageOperation::Delete
                },
                surface: StorageAccessSurface::SameOriginApplication,
                object: authority_object,
                now,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        let inventory = repository
            .authoritative_inventory(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied,
            ))?;
        if !inventory.contains_governed_object(authority_object) {
            return Err(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied,
            ));
        }
        let guard = repository
            .deletion_guard(context.clone(), subject_digest, now)
            .await
            .map_err(map_port_error)?;
        let mut workflow = repository
            .deletion_workflow(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::StateConflict)?;
        workflow.preflight_transition(&inventory, &guard, now)?;
        let expected_revision = workflow.revision();
        let target_digest = workflow.evidence_target_digest(next)?;
        let receipt = provider
            .execute_deletion_stage(
                context.clone(),
                &workflow,
                &inventory,
                next,
                &target_digest,
                now,
            )
            .await
            .map_err(map_port_error)?;
        workflow.record(&inventory, &receipt, &guard, now)?;
        match repository
            .save_deletion_workflow(
                context.clone(),
                workflow.clone(),
                Some(receipt),
                expected_revision,
                guard.authority_revision(),
            )
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            now,
            "deletion_advance",
            "verified",
            authority_object.audit_digest(),
        )
        .await?;
        Ok(workflow)
    }

    pub async fn complete_deletion(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionWorkflow, StorageGovernanceServiceError> {
        let inventory = repository
            .authoritative_inventory(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::StateConflict)?;
        let guard = repository
            .deletion_guard(context.clone(), subject_digest, now)
            .await
            .map_err(map_port_error)?;
        let mut workflow = repository
            .deletion_workflow(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::StateConflict)?;
        let expected_revision = workflow.revision();
        workflow.complete(&inventory, &guard, now)?;
        match repository
            .save_deletion_workflow(
                context.clone(),
                workflow.clone(),
                None,
                expected_revision,
                guard.authority_revision(),
            )
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            now,
            "deletion_complete",
            "complete",
            inventory.digest().clone(),
        )
        .await?;
        Ok(workflow)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn restore_deletion(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        provider: &dyn StorageGovernanceProviderV1,
        context: StorageGovernanceContextV1,
        actor: StorageActor,
        authority_object: &GovernedObject,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionWorkflow, StorageGovernanceServiceError> {
        self.authorize(
            context.correlation_id(),
            StorageAccessRequest {
                actor,
                operation: StorageOperation::Restore,
                surface: StorageAccessSurface::SameOriginApplication,
                object: authority_object,
                now,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        )?;
        let inventory = repository
            .authoritative_inventory(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::StateConflict)?;
        let guard = repository
            .deletion_guard(context.clone(), subject_digest, now)
            .await
            .map_err(map_port_error)?;
        let mut workflow = repository
            .deletion_workflow(context.clone(), subject_digest)
            .await
            .map_err(map_port_error)?
            .ok_or(StorageGovernanceServiceError::StateConflict)?;
        provider
            .verify_restore_possible(context.clone(), &workflow, &inventory)
            .await
            .map_err(map_port_error)?;
        let expected_revision = workflow.revision();
        workflow.restore(&inventory, &guard, now)?;
        match repository
            .save_deletion_workflow(
                context.clone(),
                workflow.clone(),
                None,
                expected_revision,
                guard.authority_revision(),
            )
            .await
            .map_err(map_port_error)?
        {
            StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {}
            StorageCasOutcomeV1::Conflict => {
                return Err(StorageGovernanceServiceError::StateConflict);
            }
        }
        self.append_audit(
            repository,
            context,
            now,
            "deletion_restore",
            "restored",
            authority_object.audit_digest(),
        )
        .await?;
        Ok(workflow)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reserve_quota(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        policy: StorageQuotaPolicy,
        requested_bytes: ByteSize,
        created_at: TimestampMillis,
        expires_at: TimestampMillis,
    ) -> Result<StorageQuotaReservation, StorageGovernanceServiceError> {
        for _ in 0..3 {
            let snapshot = repository
                .quota_snapshot(context.clone())
                .await
                .map_err(map_port_error)?;
            let reservation = policy.reserve_atomic(
                context.tenant_id(),
                context.correlation_id(),
                snapshot,
                requested_bytes,
                created_at,
                expires_at,
            )?;
            match repository
                .reserve_quota(context.clone(), reservation)
                .await
                .map_err(map_port_error)?
            {
                StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => {
                    return Ok(reservation);
                }
                StorageCasOutcomeV1::Conflict => {}
            }
        }
        Err(StorageGovernanceServiceError::StateConflict)
    }

    async fn append_audit(
        &self,
        repository: &dyn StorageGovernanceRepositoryV1,
        context: StorageGovernanceContextV1,
        occurred_at: TimestampMillis,
        action_code: &'static str,
        outcome_code: &'static str,
        resource_digest: ChecksumSha256,
    ) -> Result<(), StorageGovernanceServiceError> {
        for _ in 0..3 {
            let head = repository
                .audit_head(context.tenant_id())
                .await
                .map_err(map_port_error)?;
            let sequence = head.as_ref().map_or(1, |value| value.sequence() + 1);
            let previous = head
                .as_ref()
                .map_or_else(zero_checksum, |value| value.digest().clone());
            let record = DurableGovernanceAuditRecord::chained(
                sequence,
                context.tenant_id(),
                context.correlation_id(),
                context.principal_digest().clone(),
                occurred_at,
                action_code,
                outcome_code,
                resource_digest.clone(),
                previous,
            )?;
            match repository
                .append_audit(context.clone(), record)
                .await
                .map_err(map_port_error)?
            {
                StorageCasOutcomeV1::Applied | StorageCasOutcomeV1::Replay => return Ok(()),
                StorageCasOutcomeV1::Conflict => {}
            }
        }
        Err(StorageGovernanceServiceError::StateConflict)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAuthorizationReceipt {
    correlation_id: CorrelationId,
    operation: StorageOperation,
    surface: StorageAccessSurface,
    resource_digest: ChecksumSha256,
}

impl StorageAuthorizationReceipt {
    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    #[must_use]
    pub const fn operation(&self) -> StorageOperation {
        self.operation
    }

    #[must_use]
    pub const fn surface(&self) -> StorageAccessSurface {
        self.surface
    }

    #[must_use]
    pub fn resource_digest(&self) -> &ChecksumSha256 {
        &self.resource_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedObjectRead {
    receipt: StorageAuthorizationReceipt,
    policy: StorageResponsePolicy,
}

impl AuthorizedObjectRead {
    #[must_use]
    pub fn receipt(&self) -> &StorageAuthorizationReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn headers(&self) -> &BTreeMap<&'static str, String> {
        self.policy.headers()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum StorageGovernanceServiceError {
    #[error("storage request was denied")]
    Denied(StorageDenialReason),
    #[error("storage request is invalid")]
    InvalidRequest,
    #[error("storage governance configuration is invalid")]
    InvalidConfiguration,
    #[error("storage signing is unavailable")]
    SigningUnavailable,
    #[error("storage governance state changed concurrently")]
    StateConflict,
    #[error("storage governance persistence or provider is unavailable")]
    Unavailable,
    #[error("storage governance contract rejected the request")]
    Contract(StorageGovernanceError),
}

impl From<StorageGovernanceError> for StorageGovernanceServiceError {
    fn from(value: StorageGovernanceError) -> Self {
        Self::Contract(value)
    }
}

fn map_port_error(error: PortError) -> StorageGovernanceServiceError {
    match error {
        PortError::NotFound => {
            StorageGovernanceServiceError::Denied(StorageDenialReason::AccessDenied)
        }
        PortError::Conflict => StorageGovernanceServiceError::StateConflict,
        PortError::InvalidRequest(_) | PortError::Unsupported(_) => {
            StorageGovernanceServiceError::InvalidRequest
        }
        PortError::Adapter(_) => StorageGovernanceServiceError::Unavailable,
    }
}

fn zero_checksum() -> ChecksumSha256 {
    ChecksumSha256::parse("0".repeat(64)).expect("64 hexadecimal zeroes are a valid checksum")
}

fn is_canonical_configured_origin(value: &str) -> bool {
    if CorsOriginV1::parse(value).is_ok() {
        return true;
    }
    let Some(authority) = value.strip_prefix("http://") else {
        return false;
    };
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) if !port.contains(':') => (host, Some(port)),
        Some(_) => return false,
        None => (authority, None),
    };
    matches!(host, "localhost" | "127.0.0.1")
        && port.is_none_or(|port| {
            port.parse::<u16>()
                .ok()
                .is_some_and(|parsed| parsed != 0 && parsed != 80 && parsed.to_string() == port)
        })
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use frame_domain::{
        ByteSize, GovernedObjectId, GovernedObjectRole, GovernedObjectState, LifecycleObject,
        MalwareDisposition, ObjectVisibility, StorageMemberRole, UserId,
    };

    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("timestamp")
    }

    fn checksum(value: u8) -> ChecksumSha256 {
        ChecksumSha256::parse(format!("{value:064x}")).expect("checksum")
    }

    fn object(tenant_id: frame_domain::TenantId) -> GovernedObject {
        GovernedObject::new(
            tenant_id,
            GovernedObjectId::parse(format!("tenants/{tenant_id}/videos/one/source")).expect("id"),
            GovernedObjectRole::Source,
            ObjectVisibility::Private,
            GovernedObjectState::Active,
            MalwareDisposition::Clean,
            1,
            1,
            checksum(1),
            ByteSize::new(10).expect("size"),
            None,
        )
        .expect("object")
    }

    #[test]
    fn only_authorized_members_can_mint_bounded_grants() {
        let service =
            StorageGovernanceService::new(["https://app.example".to_owned()]).expect("service");
        let tenant = frame_domain::TenantId::new();
        let object = object(tenant);
        let viewer = StorageActor::Member {
            tenant_id: tenant,
            user_id: UserId::new(),
            role: StorageMemberRole::Viewer,
        };
        assert!(matches!(
            service.authorize(
                CorrelationId::new(),
                StorageAccessRequest {
                    actor: viewer,
                    operation: StorageOperation::Sign,
                    surface: StorageAccessSurface::SameOriginApplication,
                    object: &object,
                    now: timestamp(1),
                    grant: None,
                    grant_proof: None,
                    request_domain: None,
                    custom_domain: None,
                }
            ),
            Err(StorageGovernanceServiceError::Denied(_))
        ));
        let editor = StorageActor::Member {
            tenant_id: tenant,
            user_id: UserId::new(),
            role: StorageMemberRole::Editor,
        };
        service
            .authorize(
                CorrelationId::new(),
                StorageAccessRequest {
                    actor: editor,
                    operation: StorageOperation::Sign,
                    surface: StorageAccessSurface::SameOriginApplication,
                    object: &object,
                    now: timestamp(1),
                    grant: None,
                    grant_proof: None,
                    request_domain: None,
                    custom_domain: None,
                },
            )
            .expect("authorization");
    }

    #[test]
    fn deletion_and_export_are_bound_to_the_authority_tenant_and_manifest() {
        let service = StorageGovernanceService::new(Vec::new()).expect("service");
        let tenant = frame_domain::TenantId::new();
        let other = frame_domain::TenantId::new();
        let authority = object(tenant);
        let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        let objects = GovernedObjectRole::ALL
            .into_iter()
            .enumerate()
            .map(|(index, role)| LifecycleObject {
                tenant_id: other,
                object_id: GovernedObjectId::parse(format!("tenants/{other}/object-{index}"))
                    .expect("id"),
                role,
                checksum: checksum(u8::try_from(index + 1).expect("small index")),
                size: ByteSize::new(10).expect("size"),
                retention_until: None,
            })
            .collect();
        let inventory =
            LifecycleInventory::new(other, checksum(8), &required, objects).expect("inventory");
        let guard = DeletionGuardSnapshot::new(other, checksum(8), timestamp(1), 1, false, None)
            .expect("guard");
        let owner = StorageActor::Member {
            tenant_id: tenant,
            user_id: UserId::new(),
            role: StorageMemberRole::Owner,
        };
        assert!(matches!(
            service.plan_deletion(
                CorrelationId::new(),
                owner,
                &authority,
                &inventory,
                &guard,
                timestamp(1)
            ),
            Err(StorageGovernanceServiceError::Denied(
                StorageDenialReason::AccessDenied
            ))
        ));
    }

    #[test]
    fn configured_origins_are_canonical_https_or_local_http_only() {
        for invalid in [
            "http://remote.example",
            "https://app.example/path",
            "https://USER@app.example",
            "https://app.example?query",
            "HTTPS://app.example",
            "https://.example",
            "https://app.example:443",
            "https://app.example:0443",
            "http://localhost:80",
            "http://localhost:080",
        ] {
            assert_eq!(
                StorageGovernanceService::new([invalid.to_owned()]),
                Err(StorageGovernanceServiceError::InvalidConfiguration)
            );
        }
        StorageGovernanceService::new([
            "https://app.example".to_owned(),
            "http://localhost:3000".to_owned(),
        ])
        .expect("valid origins");
    }
}
