//! D1-backed storage-governance authority used by every R2-facing route.
//!
//! The adapter deliberately accepts only typed tenant context. Object keys are
//! outputs of the authority lookup; a provider prefix or listing is never an
//! authorization source.

use std::collections::BTreeSet;

use async_trait::async_trait;
use frame_application::{
    StorageGovernanceService, StorageGovernanceServiceError, StorageGrantKeyMaterialV1,
    StorageGrantKeyRingV1,
};
use frame_domain::{
    AuthorityFence, ByteSize, CacheInvalidationPlan, CachePurgeReceipt, ChecksumSha256,
    CustomDomainName, DeletionEvidenceReceipt, DeletionGuardSnapshot, DeletionStage,
    DeletionWorkflow, DurableGovernanceAuditRecord, GovernedObject, GovernedObjectId,
    GovernedObjectRole, GovernedObjectState, LifecycleInventory, LifecycleObject,
    MalwareDisposition, ManagedMediaSourcePolicy, ManagedMediaState, ObjectVisibility,
    SignedGrantId, SignedGrantKeyVersion, SignedGrantSecret, SignedObjectGrant, StorageOperation,
    StorageQuotaReservation, StorageQuotaSnapshot, TenantId, TimestampMillis, VerifiedCustomDomain,
};
use frame_ports::{
    PortError, SignedGrantSecretGeneratorV1, StorageCasOutcomeV1, StorageGovernanceContextV1,
    StorageGovernanceProviderV1, StorageGovernanceRepositoryV1,
};
use serde::{Deserialize, de::DeserializeOwned};
use wasm_bindgen::JsValue;
use worker::{Cache, D1Database, D1PreparedStatement, D1Result, Env, send::IntoSendFuture};

use crate::cutover_authority::D1CutoverAuthorityRepository;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const DEFAULT_MAX_MANAGED_INPUT_BYTES: u64 = 5 * 1_024 * 1_024 * 1_024;
const DEFAULT_MAX_MANAGED_OUTPUT_BYTES: u64 = 512 * 1_024 * 1_024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantKeyConfig {
    active: u32,
    keys: std::collections::BTreeMap<String, String>,
}

/// Builds the one application policy used by route adapters. Signing keys are
/// optional for non-grant routes, but when the secret is present every entry
/// is fully validated before the worker serves traffic.
pub fn governance_service(
    env: &Env,
    canonical_origin: &str,
) -> Result<StorageGovernanceService, StorageGovernanceServiceError> {
    let origins = storage_allowed_origins(env, canonical_origin)?;
    let Ok(secret) = env.secret("FRAME_STORAGE_GRANT_KEYS_V1") else {
        return StorageGovernanceService::new(origins);
    };
    let config = serde_json::from_str::<GrantKeyConfig>(&secret.to_string())
        .map_err(|_| StorageGovernanceServiceError::InvalidConfiguration)?;
    let active = SignedGrantKeyVersion::new(config.active)
        .map_err(|_| StorageGovernanceServiceError::InvalidConfiguration)?;
    let keys = config
        .keys
        .into_iter()
        .map(|(version, material)| {
            let version = version
                .parse::<u32>()
                .ok()
                .and_then(|value| SignedGrantKeyVersion::new(value).ok())
                .ok_or(StorageGovernanceServiceError::InvalidConfiguration)?;
            let bytes =
                decode_hex(&material).ok_or(StorageGovernanceServiceError::InvalidConfiguration)?;
            Ok((version, StorageGrantKeyMaterialV1::parse(bytes)?))
        })
        .collect::<Result<Vec<_>, StorageGovernanceServiceError>>()?;
    StorageGovernanceService::with_signing_keys(origins, StorageGrantKeyRingV1::new(active, keys)?)
}

/// Parses the exact origin allow-list used by both object responses and
/// preflight responses. Invalid configured JSON fails closed rather than
/// silently falling back to a broader origin.
pub fn storage_allowed_origins(
    env: &Env,
    canonical_origin: &str,
) -> Result<BTreeSet<String>, StorageGovernanceServiceError> {
    let origins = env
        .var("FRAME_STORAGE_ALLOWED_ORIGINS")
        .ok()
        .map(|value| {
            serde_json::from_str::<Vec<String>>(&value.to_string())
                .map_err(|_| StorageGovernanceServiceError::InvalidConfiguration)
        })
        .transpose()?
        .unwrap_or_else(|| vec![canonical_origin.to_owned()]);
    // Reuse the application constructor as the canonical validation rule.
    StorageGovernanceService::new(origins.clone())?;
    Ok(origins.into_iter().collect())
}

pub fn managed_media_policy(env: &Env) -> Result<ManagedMediaSourcePolicy, PortError> {
    let state = match env
        .var("FRAME_MANAGED_MEDIA_STATE")
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "enabled".into())
        .as_str()
    {
        "enabled" => ManagedMediaState::Enabled,
        "disabled_by_incident" => ManagedMediaState::DisabledByIncident,
        _ => return Err(PortError::Adapter("invalid managed media state".into())),
    };
    ManagedMediaSourcePolicy::new(
        state,
        ByteSize::new(DEFAULT_MAX_MANAGED_INPUT_BYTES).map_err(corrupt)?,
        ByteSize::new(DEFAULT_MAX_MANAGED_OUTPUT_BYTES).map_err(corrupt)?,
    )
    .map_err(corrupt)
}

pub fn deterministic_derivative_key(
    env: &Env,
    tenant_id: TenantId,
    video_id: &str,
    profile: &str,
    source: &GovernedObject,
) -> Result<String, PortError> {
    let input = managed_media_policy(env)?
        .authorize(tenant_id, source)
        .map_err(corrupt)?;
    let profile_digest = ChecksumSha256::digest_bytes(profile.as_bytes());
    let identity = input.deterministic_derivative_key(&profile_digest);
    Ok(format!(
        "tenants/{tenant_id}/videos/{video_id}/derivatives/{profile}/{}",
        identity.as_str()
    ))
}

pub fn deterministic_derivative_key_for_profile(
    _env: &Env,
    tenant_id: TenantId,
    video_id: &str,
    profile_id: &str,
    normalized_profile_sha256: &str,
    source: &GovernedObject,
) -> Result<String, PortError> {
    if profile_id.is_empty()
        || profile_id.len() > 64
        || !profile_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(PortError::Adapter("invalid media profile identity".into()));
    }
    let profile_digest = ChecksumSha256::parse(normalized_profile_sha256).map_err(corrupt)?;
    // Artifact identity must remain stable while the managed-provider kill
    // switch is active so native fallback resolves the same logical result.
    let identity_policy = ManagedMediaSourcePolicy::new(
        ManagedMediaState::Enabled,
        ByteSize::new(MAX_SAFE_INTEGER).map_err(corrupt)?,
        ByteSize::new(MAX_SAFE_INTEGER).map_err(corrupt)?,
    )
    .map_err(corrupt)?;
    let input = identity_policy
        .authorize(tenant_id, source)
        .map_err(corrupt)?;
    let identity = input.deterministic_derivative_key(&profile_digest);
    Ok(format!(
        "tenants/{tenant_id}/videos/{video_id}/derivatives/{profile_id}/{}",
        identity.as_str()
    ))
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeGrantSecretGenerator;

impl SignedGrantSecretGeneratorV1 for RuntimeGrantSecretGenerator {
    fn generate(&self) -> Result<SignedGrantSecret, PortError> {
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret)
            .map_err(|_| PortError::Adapter("secure random source unavailable".into()))?;
        SignedGrantSecret::parse(secret.to_vec()).map_err(corrupt)
    }
}

#[derive(Debug, Deserialize)]
struct CacheTargetRow {
    target: String,
}

/// Provider executor that mutates only the exact manifest target set, then
/// independently observes the post-state before returning typed evidence.
pub struct WorkerStorageGovernanceProvider<'runtime> {
    env: &'runtime Env,
    database: &'runtime D1Database,
    canonical_origin: String,
}

impl<'runtime> WorkerStorageGovernanceProvider<'runtime> {
    #[must_use]
    pub fn new(
        env: &'runtime Env,
        database: &'runtime D1Database,
        canonical_origin: impl Into<String>,
    ) -> Self {
        Self {
            env,
            database,
            canonical_origin: canonical_origin.into(),
        }
    }

    async fn cache_targets(
        &self,
        tenant_id: TenantId,
        object_id: &GovernedObjectId,
        generation: u64,
    ) -> Result<Vec<String>, PortError> {
        let repository = D1StorageGovernanceRepository::new(self.database);
        let rows = repository
            .rows::<CacheTargetRow>(
                "SELECT ?3 || '/api/v1/public/shares/' || v.id || '/media' AS target \
                   FROM videos v WHERE v.organization_id = ?1 AND v.playback_object_key = ?2 \
                     AND v.deleted_at_ms IS NULL \
                 UNION \
                 SELECT ?3 || '/api/v1/storage/tenants/' || g.organization_id || '/grants/' || g.grant_id \
                   FROM storage_signed_grants_v1 g \
                  WHERE g.organization_id = ?1 AND g.object_key = ?2 AND g.revoked_at_ms IS NULL",
                &[
                    JsValue::from_str(&tenant_id.to_string()),
                    JsValue::from_str(object_id.as_str()),
                    JsValue::from_str(&self.canonical_origin),
                ],
            )
            .await?;
        let mut targets = rows.into_iter().map(|row| row.target).collect::<Vec<_>>();
        let positive = targets.clone();
        targets.extend(positive.into_iter().map(|target| {
            format!("{target}?frame-cache-generation={generation}&frame-cache-variant=negative")
        }));
        targets.sort();
        targets.dedup();
        Ok(targets)
    }

    async fn purge_and_observe(&self, targets: &[String]) -> Result<(), PortError> {
        let cache = Cache::default();
        for target in targets {
            cache
                .delete(target, false)
                .into_send()
                .await
                .map_err(unavailable)?;
        }
        for target in targets {
            if cache
                .get(target, false)
                .into_send()
                .await
                .map_err(unavailable)?
                .is_some()
            {
                return Err(PortError::Adapter("cache purge observation failed".into()));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl StorageGovernanceProviderV1 for WorkerStorageGovernanceProvider<'_> {
    async fn execute_deletion_stage(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
        stage: DeletionStage,
        target_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionEvidenceReceipt, PortError> {
        if context.tenant_id() != inventory.tenant_id()
            || workflow.tenant_id() != inventory.tenant_id()
            || workflow.inventory_digest() != inventory.digest()
        {
            return Err(PortError::NotFound);
        }
        let repository = D1StorageGovernanceRepository::new(self.database);
        let mut observations = Vec::new();
        match stage {
            DeletionStage::Tombstoned => {
                for object in inventory.objects() {
                    let changes = repository
                        .run(
                            "UPDATE storage_governed_objects_v1 SET state = 'tombstoned', updated_at_ms = ?3 \
                              WHERE organization_id = ?1 AND object_key = ?2 \
                                AND checksum_sha256 = ?4 AND bytes = ?5 AND state IN ('active','tombstoned')",
                            &[
                                JsValue::from_str(&inventory.tenant_id().to_string()),
                                JsValue::from_str(object.object_id.as_str()),
                                signed_number(now.get())?,
                                JsValue::from_str(object.checksum.as_str()),
                                number(object.size.get())?,
                            ],
                        )
                        .await?;
                    if changes > 1 {
                        return Err(PortError::Adapter("tombstone cardinality mismatch".into()));
                    }
                    observations.push(format!("{}:{changes}", object.role.stable_code()));
                }
            }
            DeletionStage::OriginDeleted => {
                let bucket = self.env.bucket("RECORDINGS").map_err(unavailable)?;
                for object in inventory
                    .objects()
                    .filter(|object| object.role != GovernedObjectRole::BackupCopy)
                {
                    bucket
                        .delete(object.object_id.as_str())
                        .into_send()
                        .await
                        .map_err(unavailable)?;
                    if bucket
                        .head(object.object_id.as_str())
                        .into_send()
                        .await
                        .map_err(unavailable)?
                        .is_some()
                    {
                        return Err(PortError::Adapter(
                            "origin deletion observation failed".into(),
                        ));
                    }
                    observations.push(object.role.stable_code().to_owned());
                }
            }
            DeletionStage::CachePurged => {
                let mut targets = Vec::new();
                for object in inventory.objects() {
                    targets.extend(
                        self.cache_targets(inventory.tenant_id(), &object.object_id, 1)
                            .await?,
                    );
                }
                targets.sort();
                targets.dedup();
                self.purge_and_observe(&targets).await?;
                observations.extend(targets);
            }
            DeletionStage::BackupDeleted => {
                let bucket = self.env.bucket("BACKUPS").map_err(unavailable)?;
                for object in inventory
                    .objects()
                    .filter(|object| object.role == GovernedObjectRole::BackupCopy)
                {
                    bucket
                        .delete(object.object_id.as_str())
                        .into_send()
                        .await
                        .map_err(unavailable)?;
                    if bucket
                        .head(object.object_id.as_str())
                        .into_send()
                        .await
                        .map_err(unavailable)?
                        .is_some()
                    {
                        return Err(PortError::Adapter(
                            "backup deletion observation failed".into(),
                        ));
                    }
                    observations.push(object.role.stable_code().to_owned());
                }
            }
            DeletionStage::Verified => {
                let origin = self.env.bucket("RECORDINGS").map_err(unavailable)?;
                let backups = self.env.bucket("BACKUPS").map_err(unavailable)?;
                for object in inventory.objects() {
                    let present = if object.role == GovernedObjectRole::BackupCopy {
                        backups
                            .head(object.object_id.as_str())
                            .into_send()
                            .await
                            .map_err(unavailable)?
                            .is_some()
                    } else {
                        origin
                            .head(object.object_id.as_str())
                            .into_send()
                            .await
                            .map_err(unavailable)?
                            .is_some()
                    };
                    if present {
                        return Err(PortError::Adapter("erasure observation failed".into()));
                    }
                    observations.push(object.role.stable_code().to_owned());
                }
            }
            DeletionStage::Planned | DeletionStage::Complete | DeletionStage::Restored => {
                return Err(PortError::InvalidRequest(
                    "invalid provider deletion stage".into(),
                ));
            }
        }
        let provider_receipt_digest =
            ChecksumSha256::digest_bytes(observations.join("\0").as_bytes());
        let cache_stage = stage == DeletionStage::CachePurged;
        DeletionEvidenceReceipt::verified(
            inventory.tenant_id(),
            inventory.digest().clone(),
            stage,
            target_digest.clone(),
            provider_receipt_digest,
            now,
            cache_stage,
            cache_stage,
        )
        .map_err(corrupt)
    }

    async fn verify_restore_possible(
        &self,
        context: StorageGovernanceContextV1,
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
    ) -> Result<(), PortError> {
        if context.tenant_id() != inventory.tenant_id()
            || workflow.tenant_id() != inventory.tenant_id()
            || workflow.inventory_digest() != inventory.digest()
            || !matches!(
                workflow.stage(),
                DeletionStage::Planned | DeletionStage::Tombstoned | DeletionStage::Restored
            )
        {
            return Err(PortError::NotFound);
        }
        let origin = self.env.bucket("RECORDINGS").map_err(unavailable)?;
        let backups = self.env.bucket("BACKUPS").map_err(unavailable)?;
        for object in inventory.objects() {
            let head = if object.role == GovernedObjectRole::BackupCopy {
                backups
                    .head(object.object_id.as_str())
                    .into_send()
                    .await
                    .map_err(unavailable)?
            } else {
                origin
                    .head(object.object_id.as_str())
                    .into_send()
                    .await
                    .map_err(unavailable)?
            };
            let Some(head) = head else {
                return Err(PortError::Conflict);
            };
            let checksum = decode_hex(object.checksum.as_str())
                .ok_or_else(|| PortError::Adapter("invalid restore checksum".into()))?;
            if head.size() != object.size.get()
                || head.checksum().sha256.as_deref() != Some(checksum.as_slice())
            {
                return Err(PortError::Conflict);
            }
        }
        Ok(())
    }

    async fn purge_and_probe_cache(
        &self,
        context: StorageGovernanceContextV1,
        plan: &CacheInvalidationPlan,
        now: TimestampMillis,
    ) -> Result<CachePurgeReceipt, PortError> {
        if context.tenant_id() != plan.tenant_id() {
            return Err(PortError::NotFound);
        }
        let targets = self
            .cache_targets(plan.tenant_id(), plan.object_id(), plan.from_generation())
            .await?;
        self.purge_and_observe(&targets).await?;
        CachePurgeReceipt::verified_absence(
            plan.tenant_id(),
            plan.object_id().clone(),
            plan.from_generation(),
            plan.cache_tag_digest(),
            ChecksumSha256::digest_bytes(targets.join("\0").as_bytes()),
            now,
            true,
            true,
        )
        .map_err(corrupt)
    }
}

#[derive(Debug, Deserialize)]
struct GovernedObjectRow {
    organization_id: String,
    object_key: String,
    role: String,
    visibility: String,
    state: String,
    malware_disposition: String,
    immutable_revision: i64,
    cache_generation: i64,
    checksum_sha256: String,
    bytes: i64,
    retention_until_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct JsonRow {
    value_json: String,
}

#[derive(Debug, Deserialize)]
struct DomainRow {
    organization_id: String,
    verification_version: i64,
    active: i64,
}

#[derive(Debug, Deserialize)]
struct ManifestRow {
    authority_revision: i64,
    retention_until_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LifecycleObjectRow {
    organization_id: String,
    object_key: String,
    role: String,
    checksum_sha256: String,
    bytes: i64,
    retention_until_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct QuotaRow {
    used_bytes: i64,
    reserved_bytes: i64,
    used_objects: i64,
    reserved_objects: i64,
    revision: i64,
}

/// D1 implementation of the durable storage-governance port.
pub struct D1StorageGovernanceRepository<'database> {
    database: &'database D1Database,
    cutover: Option<CutoverMutationFence>,
}

struct CutoverMutationFence {
    authority: AuthorityFence,
    occurred_at: TimestampMillis,
    operation_prefix: String,
}

impl<'database> D1StorageGovernanceRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self {
            database,
            cutover: None,
        }
    }

    pub fn with_cutover_fence(
        database: &'database D1Database,
        authority: Option<AuthorityFence>,
        occurred_at: TimestampMillis,
        operation_prefix: impl Into<String>,
    ) -> Result<Self, PortError> {
        let operation_prefix = operation_prefix.into();
        if operation_prefix.is_empty()
            || operation_prefix.len() > 150
            || !operation_prefix.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b':' | b'/' | b'@' | b'+' | b'-')
            })
        {
            return Err(PortError::InvalidRequest(
                "invalid cutover operation prefix".into(),
            ));
        }
        Ok(Self {
            database,
            cutover: authority.map(|authority| CutoverMutationFence {
                authority,
                occurred_at,
                operation_prefix,
            }),
        })
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> Result<D1PreparedStatement, PortError> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(unavailable)
    }

    async fn first<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Option<T>, PortError> {
        self.statement(sql, bindings)?
            .first::<T>(None)
            .into_send()
            .await
            .map_err(unavailable)
    }

    async fn rows<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Vec<T>, PortError> {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(unavailable)?;
        if !result.success() {
            return Err(PortError::Adapter("storage authority query failed".into()));
        }
        result.results::<T>().map_err(unavailable)
    }

    async fn run(&self, sql: &str, bindings: &[JsValue]) -> Result<usize, PortError> {
        let statement = self.statement(sql, bindings)?;
        let result = if let Some(cutover) = self.cutover.as_ref() {
            D1CutoverAuthorityRepository::new(self.database)
                .execute_fenced_batch_results(
                    &format!("{}:run", cutover.operation_prefix),
                    &cutover.authority,
                    cutover.occurred_at,
                    vec![statement],
                )
                .await
                .map_err(|_| PortError::Adapter("scoped storage mutation rejected".into()))?
                .into_iter()
                .next()
                .ok_or_else(|| PortError::Adapter("storage authority mutation failed".into()))?
        } else {
            statement.run().into_send().await.map_err(unavailable)?
        };
        result_changes(&result)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> Result<Vec<usize>, PortError> {
        let results = if let Some(cutover) = self.cutover.as_ref() {
            D1CutoverAuthorityRepository::new(self.database)
                .execute_fenced_batch_results(
                    &format!("{}:batch", cutover.operation_prefix),
                    &cutover.authority,
                    cutover.occurred_at,
                    statements,
                )
                .await
                .map_err(|_| PortError::Adapter("scoped storage mutation rejected".into()))?
        } else {
            self.database
                .batch(statements)
                .into_send()
                .await
                .map_err(unavailable)?
        };
        if results.is_empty() || results.iter().any(|result| !result.success()) {
            return Err(PortError::Adapter("storage authority batch failed".into()));
        }
        results.iter().map(result_changes).collect()
    }

    fn validate_context(
        context: &StorageGovernanceContextV1,
        tenant_id: TenantId,
    ) -> Result<(), PortError> {
        if context.tenant_id() == tenant_id {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }
}

#[async_trait]
impl StorageGovernanceRepositoryV1 for D1StorageGovernanceRepository<'_> {
    async fn governed_object(
        &self,
        context: StorageGovernanceContextV1,
        object_id: &GovernedObjectId,
    ) -> Result<Option<GovernedObject>, PortError> {
        let tenant = context.tenant_id();
        let row = self
            .first::<GovernedObjectRow>(
                "SELECT organization_id, object_key, role, visibility, state, malware_disposition, \
                        immutable_revision, cache_generation, checksum_sha256, bytes, retention_until_ms \
                 FROM storage_governed_objects_v1 \
                 WHERE organization_id = ?1 AND object_key = ?2 LIMIT 1",
                &[
                    JsValue::from_str(&tenant.to_string()),
                    JsValue::from_str(object_id.as_str()),
                ],
            )
            .await?;
        row.map(parse_governed_object).transpose()
    }

    async fn insert_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant: SignedObjectGrant,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, grant.tenant_id())?;
        let grant_json = serde_json::to_string(&grant).map_err(corrupt)?;
        let changes = self
            .run(
                "INSERT OR IGNORE INTO storage_signed_grants_v1( \
                   grant_id, organization_id, object_key, key_version, operation, issued_at_ms, \
                   expires_at_ms, nonce_digest, revoked_at_ms, grant_json \
                 ) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9 \
                   FROM storage_governed_objects_v1 g \
                  WHERE g.organization_id = ?2 AND g.object_key = ?3",
                &[
                    JsValue::from_str(&grant.grant_id().to_string()),
                    JsValue::from_str(&grant.tenant_id().to_string()),
                    JsValue::from_str(grant.object_id().as_str()),
                    number(u64::from(grant.key_version().get()))?,
                    JsValue::from_str(operation_code(grant.operation())),
                    signed_number(grant.issued_at().get())?,
                    signed_number(grant.expires_at().get())?,
                    JsValue::from_str(grant.nonce_digest().as_str()),
                    JsValue::from_str(&grant_json),
                ],
            )
            .await?;
        if changes == 1 {
            Ok(StorageCasOutcomeV1::Applied)
        } else {
            let existing = self.signed_grant(context, grant.grant_id()).await?;
            Ok(if existing.as_ref() == Some(&grant) {
                StorageCasOutcomeV1::Replay
            } else {
                StorageCasOutcomeV1::Conflict
            })
        }
    }

    async fn signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
    ) -> Result<Option<SignedObjectGrant>, PortError> {
        let row = self
            .first::<JsonRow>(
                "SELECT grant_json AS value_json FROM storage_signed_grants_v1 \
                 WHERE grant_id = ?1 AND organization_id = ?2 LIMIT 1",
                &[
                    JsValue::from_str(&grant_id.to_string()),
                    JsValue::from_str(&context.tenant_id().to_string()),
                ],
            )
            .await?;
        row.map(|row| serde_json::from_str(&row.value_json).map_err(corrupt))
            .transpose()
    }

    async fn revoke_signed_grant(
        &self,
        context: StorageGovernanceContextV1,
        grant_id: SignedGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        let Some(mut grant) = self.signed_grant(context.clone(), grant_id).await? else {
            return Err(PortError::NotFound);
        };
        if grant.revoked_at() == Some(revoked_at) {
            return Ok(StorageCasOutcomeV1::Replay);
        }
        grant.revoke(revoked_at).map_err(corrupt)?;
        let grant_json = serde_json::to_string(&grant).map_err(corrupt)?;
        let changes = self
            .run(
                "UPDATE storage_signed_grants_v1 \
                    SET revoked_at_ms = ?3, grant_json = ?4 \
                  WHERE grant_id = ?1 AND organization_id = ?2 AND revoked_at_ms IS NULL",
                &[
                    JsValue::from_str(&grant_id.to_string()),
                    JsValue::from_str(&context.tenant_id().to_string()),
                    signed_number(revoked_at.get())?,
                    JsValue::from_str(&grant_json),
                ],
            )
            .await?;
        Ok(if changes == 1 {
            StorageCasOutcomeV1::Applied
        } else {
            StorageCasOutcomeV1::Conflict
        })
    }

    async fn verified_domain(
        &self,
        domain: &str,
    ) -> Result<Option<VerifiedCustomDomain>, PortError> {
        let domain = CustomDomainName::parse(domain).map_err(corrupt)?;
        let row = self
            .first::<DomainRow>(
                "SELECT MIN(organization_id) AS organization_id, \
                        MIN(verification_version) AS verification_version, MIN(active) AS active \
                   FROM storage_verified_domains_v1 WHERE domain_ascii = ?1 \
                  HAVING COUNT(*) = 1",
                &[JsValue::from_str(domain.as_str())],
            )
            .await?;
        row.map(|row| {
            VerifiedCustomDomain::new(
                parse_tenant(&row.organization_id)?,
                domain,
                safe_u64(row.verification_version)?,
                row.active == 1,
            )
            .map_err(corrupt)
        })
        .transpose()
    }

    async fn authoritative_inventory(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<LifecycleInventory>, PortError> {
        let tenant = context.tenant_id();
        let Some(_manifest) = self
            .first::<ManifestRow>(
                "SELECT authority_revision, retention_until_ms \
                   FROM storage_lifecycle_manifests_v1 \
                  WHERE organization_id = ?1 AND subject_digest = ?2 LIMIT 1",
                &[
                    JsValue::from_str(&tenant.to_string()),
                    JsValue::from_str(subject_digest.as_str()),
                ],
            )
            .await?
        else {
            return Ok(None);
        };
        let rows = self
            .rows::<LifecycleObjectRow>(
                "SELECT organization_id, object_key, role, checksum_sha256, bytes, retention_until_ms \
                   FROM storage_lifecycle_manifest_objects_v1 \
                  WHERE organization_id = ?1 AND subject_digest = ?2 ORDER BY object_key",
                &[
                    JsValue::from_str(&tenant.to_string()),
                    JsValue::from_str(subject_digest.as_str()),
                ],
            )
            .await?;
        let objects = rows
            .into_iter()
            .map(parse_lifecycle_object)
            .collect::<Result<Vec<_>, _>>()?;
        let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        LifecycleInventory::new(tenant, subject_digest.clone(), &required, objects)
            .map(Some)
            .map_err(corrupt)
    }

    async fn deletion_guard(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<DeletionGuardSnapshot, PortError> {
        let tenant = context.tenant_id();
        let manifest = self
            .first::<ManifestRow>(
                "SELECT authority_revision, retention_until_ms \
                   FROM storage_lifecycle_manifests_v1 \
                  WHERE organization_id = ?1 AND subject_digest = ?2 LIMIT 1",
                &[
                    JsValue::from_str(&tenant.to_string()),
                    JsValue::from_str(subject_digest.as_str()),
                ],
            )
            .await?
            .ok_or(PortError::NotFound)?;
        let active_hold = self
            .first::<CountRow>(
                "SELECT COUNT(*) AS row_count FROM storage_active_hold_bridge_v1 \
                  WHERE organization_id = ?1 AND subject_digest = ?2",
                &[
                    JsValue::from_str(&tenant.to_string()),
                    JsValue::from_str(subject_digest.as_str()),
                ],
            )
            .await?
            .is_some_and(|row| row.row_count > 0);
        DeletionGuardSnapshot::new(
            tenant,
            subject_digest.clone(),
            now,
            safe_u64(manifest.authority_revision)?,
            active_hold,
            manifest
                .retention_until_ms
                .map(parse_timestamp)
                .transpose()?,
        )
        .map_err(corrupt)
    }

    async fn create_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, workflow.tenant_id())?;
        let json = serde_json::to_string(&workflow).map_err(corrupt)?;
        let changes = self
            .run(
                "INSERT OR IGNORE INTO storage_deletion_workflows_v1( \
                   organization_id, subject_digest, correlation_id, inventory_digest, stage, \
                   revision, guard_revision, workflow_json, requested_at_ms, updated_at_ms \
                 ) SELECT ?1, ?2, ?3, ?4, 'planned', ?5, ?6, ?7, ?8, ?8 \
                   FROM storage_lifecycle_manifests_v1 m \
                  WHERE m.organization_id = ?1 AND m.subject_digest = ?2 \
                    AND m.authority_revision = ?6 \
                    AND (m.retention_until_ms IS NULL OR m.retention_until_ms <= ?8) \
                    AND NOT EXISTS (SELECT 1 FROM storage_active_hold_bridge_v1 h \
                                     WHERE h.organization_id = ?1 AND h.subject_digest = ?2) \
                    AND NOT EXISTS (SELECT 1 FROM storage_lifecycle_manifest_objects_v1 o \
                                     WHERE o.organization_id = ?1 AND o.subject_digest = ?2 \
                                       AND o.retention_until_ms > ?8)",
                &[
                    JsValue::from_str(&workflow.tenant_id().to_string()),
                    JsValue::from_str(workflow.subject_digest().as_str()),
                    JsValue::from_str(&workflow.correlation_id().to_string()),
                    JsValue::from_str(workflow.inventory_digest().as_str()),
                    number(workflow.revision())?,
                    number(expected_guard_revision)?,
                    JsValue::from_str(&json),
                    signed_number(workflow.requested_at().get())?,
                ],
            )
            .await?;
        if changes == 1 {
            Ok(StorageCasOutcomeV1::Applied)
        } else {
            let existing = self
                .deletion_workflow(context, workflow.subject_digest())
                .await?;
            Ok(if existing.as_ref() == Some(&workflow) {
                StorageCasOutcomeV1::Replay
            } else {
                StorageCasOutcomeV1::Conflict
            })
        }
    }

    async fn deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        subject_digest: &ChecksumSha256,
    ) -> Result<Option<DeletionWorkflow>, PortError> {
        let row = self
            .first::<JsonRow>(
                "SELECT workflow_json AS value_json FROM storage_deletion_workflows_v1 \
                  WHERE organization_id = ?1 AND subject_digest = ?2 LIMIT 1",
                &[
                    JsValue::from_str(&context.tenant_id().to_string()),
                    JsValue::from_str(subject_digest.as_str()),
                ],
            )
            .await?;
        row.map(|row| serde_json::from_str(&row.value_json).map_err(corrupt))
            .transpose()
    }

    async fn save_deletion_workflow(
        &self,
        context: StorageGovernanceContextV1,
        workflow: DeletionWorkflow,
        evidence: Option<DeletionEvidenceReceipt>,
        expected_workflow_revision: u64,
        expected_guard_revision: u64,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, workflow.tenant_id())?;
        let workflow_json = serde_json::to_string(&workflow).map_err(corrupt)?;
        let updated_at = evidence.as_ref().map_or_else(
            || workflow.requested_at(),
            DeletionEvidenceReceipt::observed_at,
        );
        let update = self.statement(
            "UPDATE storage_deletion_workflows_v1 \
                SET stage = ?3, revision = ?4, guard_revision = ?5, workflow_json = ?6, \
                    updated_at_ms = ?7 \
              WHERE organization_id = ?1 AND subject_digest = ?2 AND revision = ?8 \
                AND guard_revision = ?5 \
                AND EXISTS (SELECT 1 FROM storage_lifecycle_manifests_v1 m \
                             WHERE m.organization_id = ?1 AND m.subject_digest = ?2 \
                               AND m.authority_revision = ?5 \
                               AND (?3 = 'restored' OR m.retention_until_ms IS NULL \
                                    OR m.retention_until_ms <= ?7)) \
                AND (?3 = 'restored' OR NOT EXISTS \
                    (SELECT 1 FROM storage_active_hold_bridge_v1 h \
                      WHERE h.organization_id = ?1 AND h.subject_digest = ?2)) \
                AND (?3 = 'restored' OR NOT EXISTS \
                    (SELECT 1 FROM storage_lifecycle_manifest_objects_v1 o \
                      WHERE o.organization_id = ?1 AND o.subject_digest = ?2 \
                        AND o.retention_until_ms > ?7))",
            &[
                JsValue::from_str(&workflow.tenant_id().to_string()),
                JsValue::from_str(workflow.subject_digest().as_str()),
                JsValue::from_str(stage_code(workflow.stage())),
                number(workflow.revision())?,
                number(expected_guard_revision)?,
                JsValue::from_str(&workflow_json),
                signed_number(updated_at.get())?,
                number(expected_workflow_revision)?,
            ],
        )?;
        let mut statements = vec![update];
        if workflow.stage() == DeletionStage::Restored {
            statements.push(self.statement(
                "UPDATE storage_governed_objects_v1 \
                    SET state = 'active', updated_at_ms = ?5 \
                  WHERE organization_id = ?1 AND state = 'tombstoned' \
                    AND EXISTS (SELECT 1 FROM storage_lifecycle_manifest_objects_v1 o \
                                 WHERE o.organization_id = ?1 AND o.subject_digest = ?2 \
                                   AND o.object_key = storage_governed_objects_v1.object_key \
                                   AND o.checksum_sha256 = storage_governed_objects_v1.checksum_sha256 \
                                   AND o.bytes = storage_governed_objects_v1.bytes) \
                    AND EXISTS (SELECT 1 FROM storage_deletion_workflows_v1 w \
                                 WHERE w.organization_id = ?1 AND w.subject_digest = ?2 \
                                   AND w.correlation_id = ?3 AND w.stage = 'restored' \
                                   AND w.revision = ?4)",
                &[
                    JsValue::from_str(&workflow.tenant_id().to_string()),
                    JsValue::from_str(workflow.subject_digest().as_str()),
                    JsValue::from_str(&workflow.correlation_id().to_string()),
                    number(workflow.revision())?,
                    signed_number(updated_at.get())?,
                ],
            )?);
        }
        if let Some(receipt) = evidence.as_ref() {
            let receipt_json = serde_json::to_string(receipt).map_err(corrupt)?;
            statements.push(self.statement(
                "INSERT OR IGNORE INTO storage_deletion_evidence_v1( \
                   correlation_id, stage, target_digest, provider_receipt_digest, observed_at_ms, receipt_json \
                 ) SELECT ?1, ?2, ?3, ?4, ?5, ?6 \
                    WHERE EXISTS (SELECT 1 FROM storage_deletion_workflows_v1 w \
                                   WHERE w.correlation_id = ?1 AND w.revision = ?7 AND w.stage = ?2)",
                &[
                    JsValue::from_str(&workflow.correlation_id().to_string()),
                    JsValue::from_str(stage_code(receipt.stage())),
                    JsValue::from_str(receipt.target_digest().as_str()),
                    JsValue::from_str(receipt.provider_receipt_digest().as_str()),
                    signed_number(receipt.observed_at().get())?,
                    JsValue::from_str(&receipt_json),
                    number(workflow.revision())?,
                ],
            )?);
        }
        let changes = self.batch(statements).await?;
        let applied =
            changes.first() == Some(&1) && (evidence.is_none() || changes.get(1) == Some(&1));
        if applied {
            return Ok(StorageCasOutcomeV1::Applied);
        }
        let current = self
            .deletion_workflow(context, workflow.subject_digest())
            .await?;
        Ok(if current.as_ref() == Some(&workflow) {
            StorageCasOutcomeV1::Replay
        } else {
            StorageCasOutcomeV1::Conflict
        })
    }

    async fn quota_snapshot(
        &self,
        context: StorageGovernanceContextV1,
    ) -> Result<StorageQuotaSnapshot, PortError> {
        let tenant = context.tenant_id();
        let row = self
            .first::<QuotaRow>(
                "SELECT q.used_bytes, q.used_objects, q.revision, \
                        COALESCE(SUM(CASE WHEN r.state = 'outstanding' THEN r.requested_bytes ELSE 0 END), 0) \
                          AS reserved_bytes, \
                        COALESCE(SUM(CASE WHEN r.state = 'outstanding' THEN 1 ELSE 0 END), 0) \
                          AS reserved_objects \
                   FROM storage_quota_state_v1 q \
                   LEFT JOIN storage_quota_reservations_v1 r ON r.organization_id = q.organization_id \
                  WHERE q.organization_id = ?1 GROUP BY q.organization_id LIMIT 1",
                &[JsValue::from_str(&tenant.to_string())],
            )
            .await?
            .ok_or(PortError::NotFound)?;
        StorageQuotaSnapshot::new(
            tenant,
            ByteSize::new(safe_u64(row.used_bytes)?).map_err(corrupt)?,
            ByteSize::new(safe_u64(row.reserved_bytes)?).map_err(corrupt)?,
            safe_u64(row.used_objects)?,
            safe_u64(row.reserved_objects)?,
            safe_u64(row.revision)?,
        )
        .map_err(corrupt)
    }

    async fn reserve_quota(
        &self,
        context: StorageGovernanceContextV1,
        reservation: StorageQuotaReservation,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, reservation.tenant_id())?;
        let tenant = reservation.tenant_id().to_string();
        let reservation_id = reservation.reservation_id().to_string();
        let insert = self.statement(
            "INSERT OR IGNORE INTO storage_quota_reservations_v1( \
               reservation_id, organization_id, requested_bytes, state, expected_quota_revision, \
               created_at_ms, expires_at_ms, completed_at_ms \
             ) SELECT ?1, ?2, ?3, 'outstanding', ?4, ?5, ?6, NULL \
                 FROM storage_quota_state_v1 q \
                WHERE q.organization_id = ?2 AND q.revision = ?4 \
                  AND q.used_bytes + COALESCE((SELECT SUM(r.requested_bytes) \
                       FROM storage_quota_reservations_v1 r \
                       WHERE r.organization_id = ?2 AND r.state = 'outstanding'), 0) + ?3 <= q.max_bytes \
                  AND q.used_objects + COALESCE((SELECT COUNT(*) \
                       FROM storage_quota_reservations_v1 r \
                       WHERE r.organization_id = ?2 AND r.state = 'outstanding'), 0) + 1 <= q.max_objects",
            &[
                JsValue::from_str(&reservation_id),
                JsValue::from_str(&tenant),
                number(reservation.requested_bytes().get())?,
                number(reservation.expected_quota_revision())?,
                signed_number(reservation.created_at().get())?,
                signed_number(reservation.expires_at().get())?,
            ],
        )?;
        let update = self.statement(
            "UPDATE storage_quota_state_v1 SET revision = revision + 1, updated_at_ms = ?5 \
              WHERE organization_id = ?2 AND revision = ?4 \
                AND EXISTS (SELECT 1 FROM storage_quota_reservations_v1 r \
                             WHERE r.reservation_id = ?1 AND r.organization_id = ?2 \
                               AND r.expected_quota_revision = ?4 AND r.state = 'outstanding')",
            &[
                JsValue::from_str(&reservation_id),
                JsValue::from_str(&tenant),
                number(reservation.requested_bytes().get())?,
                number(reservation.expected_quota_revision())?,
                signed_number(reservation.created_at().get())?,
            ],
        )?;
        let changes = self.batch(vec![insert, update]).await?;
        Ok(match changes.as_slice() {
            [1, 1] => StorageCasOutcomeV1::Applied,
            [0, 0] => StorageCasOutcomeV1::Conflict,
            _ => return Err(PortError::Adapter("partial quota reservation".into())),
        })
    }

    async fn release_quota_reservation(
        &self,
        context: StorageGovernanceContextV1,
        reservation_id: frame_domain::CorrelationId,
        committed: bool,
        completed_at: TimestampMillis,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        let tenant = context.tenant_id().to_string();
        let reservation_id = reservation_id.to_string();
        let state = if committed { "committed" } else { "released" };
        let quota_update = self.statement(
            "UPDATE storage_quota_state_v1 \
                SET used_bytes = used_bytes + CASE WHEN ?3 = 'committed' THEN \
                      (SELECT requested_bytes FROM storage_quota_reservations_v1 WHERE reservation_id = ?1) ELSE 0 END, \
                    used_objects = used_objects + CASE WHEN ?3 = 'committed' THEN 1 ELSE 0 END, \
                    revision = revision + 1, updated_at_ms = ?4 \
              WHERE organization_id = ?2 \
                AND EXISTS (SELECT 1 FROM storage_quota_reservations_v1 r \
                             WHERE r.reservation_id = ?1 AND r.organization_id = ?2 \
                               AND r.state = 'outstanding')",
            &[
                JsValue::from_str(&reservation_id),
                JsValue::from_str(&tenant),
                JsValue::from_str(state),
                signed_number(completed_at.get())?,
            ],
        )?;
        let reservation_update = self.statement(
            "UPDATE storage_quota_reservations_v1 SET state = ?3, completed_at_ms = ?4 \
              WHERE reservation_id = ?1 AND organization_id = ?2 AND state = 'outstanding'",
            &[
                JsValue::from_str(&reservation_id),
                JsValue::from_str(&tenant),
                JsValue::from_str(state),
                signed_number(completed_at.get())?,
            ],
        )?;
        let changes = self.batch(vec![quota_update, reservation_update]).await?;
        Ok(match changes.as_slice() {
            [1, 1] => StorageCasOutcomeV1::Applied,
            [0, 0] => StorageCasOutcomeV1::Replay,
            _ => return Err(PortError::Adapter("partial quota release".into())),
        })
    }

    async fn begin_privacy_transition(
        &self,
        context: StorageGovernanceContextV1,
        object: &GovernedObject,
        plan: CacheInvalidationPlan,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, object.tenant_id())?;
        let plan_json = serde_json::to_string(&plan).map_err(corrupt)?;
        let plan_digest = plan.cache_tag_digest();
        let tenant = object.tenant_id().to_string();
        let insert = self.statement(
            "INSERT OR IGNORE INTO storage_cache_operations_v1( \
               plan_digest, organization_id, object_key, from_generation, to_generation, deadline_ms, \
               state, plan_json, receipt_json, positive_absent, negative_absent, verified_at_ms \
             ) SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, NULL, NULL, NULL, NULL \
                 FROM storage_governed_objects_v1 g \
                WHERE g.organization_id = ?2 AND g.object_key = ?3 \
                  AND g.immutable_revision = ?8 AND g.cache_generation = ?4 \
                  AND g.checksum_sha256 = ?9",
            &[
                JsValue::from_str(plan_digest.as_str()),
                JsValue::from_str(&tenant),
                JsValue::from_str(object.object_id().as_str()),
                number(plan.from_generation())?,
                number(plan.to_generation())?,
                signed_number(plan.deadline().get())?,
                JsValue::from_str(&plan_json),
                number(object.immutable_revision())?,
                JsValue::from_str(object.checksum().as_str()),
            ],
        )?;
        let update = self.statement(
            "UPDATE storage_governed_objects_v1 \
                SET visibility = json_extract(?7, '$.to_visibility'), cache_generation = ?5, \
                    updated_at_ms = json_extract(?7, '$.changed_at') \
              WHERE organization_id = ?2 AND object_key = ?3 \
                AND immutable_revision = ?8 AND cache_generation = ?4 AND checksum_sha256 = ?9 \
                AND EXISTS (SELECT 1 FROM storage_cache_operations_v1 c \
                             WHERE c.plan_digest = ?1 AND c.state = 'pending')",
            &[
                JsValue::from_str(plan_digest.as_str()),
                JsValue::from_str(&tenant),
                JsValue::from_str(object.object_id().as_str()),
                number(plan.from_generation())?,
                number(plan.to_generation())?,
                signed_number(plan.deadline().get())?,
                JsValue::from_str(&plan_json),
                number(object.immutable_revision())?,
                JsValue::from_str(object.checksum().as_str()),
            ],
        )?;
        let changes = self.batch(vec![insert, update]).await?;
        Ok(match changes.as_slice() {
            [1, 1] => StorageCasOutcomeV1::Applied,
            [0, 0] => StorageCasOutcomeV1::Replay,
            _ => return Err(PortError::Adapter("partial privacy transition".into())),
        })
    }

    async fn complete_cache_plan(
        &self,
        context: StorageGovernanceContextV1,
        receipt: CachePurgeReceipt,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        let receipt_json = serde_json::to_string(&receipt).map_err(corrupt)?;
        let changes = self
            .run(
                "UPDATE storage_cache_operations_v1 \
                    SET state = 'verified', receipt_json = ?3, positive_absent = 1, \
                        negative_absent = 1, verified_at_ms = ?4 \
                  WHERE organization_id = ?1 AND plan_digest = json_extract(?3, '$.plan_digest') \
                    AND state = 'pending' AND deadline_ms >= ?4",
                &[
                    JsValue::from_str(&context.tenant_id().to_string()),
                    JsValue::from_str("reserved"),
                    JsValue::from_str(&receipt_json),
                    signed_number(receipt.observed_at().get())?,
                ],
            )
            .await?;
        Ok(if changes == 1 {
            StorageCasOutcomeV1::Applied
        } else {
            StorageCasOutcomeV1::Conflict
        })
    }

    async fn audit_head(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<DurableGovernanceAuditRecord>, PortError> {
        let row = self
            .first::<JsonRow>(
                "SELECT record_json AS value_json FROM storage_governance_audit_v1 \
                  WHERE organization_id = ?1 ORDER BY sequence DESC LIMIT 1",
                &[JsValue::from_str(&tenant_id.to_string())],
            )
            .await?;
        row.map(|row| serde_json::from_str(&row.value_json).map_err(corrupt))
            .transpose()
    }

    async fn append_audit(
        &self,
        context: StorageGovernanceContextV1,
        record: DurableGovernanceAuditRecord,
    ) -> Result<StorageCasOutcomeV1, PortError> {
        Self::validate_context(&context, record.tenant_id())?;
        if !record.verify() {
            return Err(PortError::InvalidRequest(
                "invalid audit chain record".into(),
            ));
        }
        let json = serde_json::to_string(&record).map_err(corrupt)?;
        let changes = self
            .run(
                "INSERT OR IGNORE INTO storage_governance_audit_v1( \
                   organization_id, sequence, correlation_id, previous_digest, digest, record_json, occurred_at_ms \
                 ) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 \
                    WHERE ?2 = COALESCE((SELECT MAX(a.sequence) + 1 FROM storage_governance_audit_v1 a \
                                         WHERE a.organization_id = ?1), 1) \
                      AND ?4 = COALESCE((SELECT a.digest FROM storage_governance_audit_v1 a \
                                         WHERE a.organization_id = ?1 ORDER BY a.sequence DESC LIMIT 1), \
                                        '0000000000000000000000000000000000000000000000000000000000000000') \
                      AND ?7 >= COALESCE((SELECT a.occurred_at_ms FROM storage_governance_audit_v1 a \
                                          WHERE a.organization_id = ?1 ORDER BY a.sequence DESC LIMIT 1), 0)",
                &[
                    JsValue::from_str(&record.tenant_id().to_string()),
                    number(record.sequence())?,
                    JsValue::from_str(&record.correlation_id().to_string()),
                    JsValue::from_str(record.previous_digest().as_str()),
                    JsValue::from_str(record.digest().as_str()),
                    JsValue::from_str(&json),
                    signed_number(record.occurred_at().get())?,
                ],
            )
            .await?;
        if changes == 1 {
            return Ok(StorageCasOutcomeV1::Applied);
        }
        let existing = self.audit_head(record.tenant_id()).await?;
        Ok(if existing.as_ref() == Some(&record) {
            StorageCasOutcomeV1::Replay
        } else {
            StorageCasOutcomeV1::Conflict
        })
    }
}

#[derive(Debug, Deserialize)]
struct CountRow {
    row_count: i64,
}

fn parse_governed_object(row: GovernedObjectRow) -> Result<GovernedObject, PortError> {
    GovernedObject::new(
        parse_tenant(&row.organization_id)?,
        GovernedObjectId::parse(row.object_key).map_err(corrupt)?,
        parse_role(&row.role)?,
        parse_visibility(&row.visibility)?,
        parse_state(&row.state)?,
        parse_malware(&row.malware_disposition)?,
        safe_u64(row.immutable_revision)?,
        safe_u64(row.cache_generation)?,
        ChecksumSha256::parse(row.checksum_sha256).map_err(corrupt)?,
        ByteSize::new(safe_u64(row.bytes)?).map_err(corrupt)?,
        row.retention_until_ms.map(parse_timestamp).transpose()?,
    )
    .map_err(corrupt)
}

fn parse_lifecycle_object(row: LifecycleObjectRow) -> Result<LifecycleObject, PortError> {
    Ok(LifecycleObject {
        tenant_id: parse_tenant(&row.organization_id)?,
        object_id: GovernedObjectId::parse(row.object_key).map_err(corrupt)?,
        role: parse_role(&row.role)?,
        checksum: ChecksumSha256::parse(row.checksum_sha256).map_err(corrupt)?,
        size: ByteSize::new(safe_u64(row.bytes)?).map_err(corrupt)?,
        retention_until: row.retention_until_ms.map(parse_timestamp).transpose()?,
    })
}

fn parse_role(value: &str) -> Result<GovernedObjectRole, PortError> {
    GovernedObjectRole::ALL
        .into_iter()
        .find(|role| role.stable_code() == value)
        .ok_or_else(|| PortError::Adapter("invalid governed object role".into()))
}

fn parse_visibility(value: &str) -> Result<ObjectVisibility, PortError> {
    match value {
        "private" => Ok(ObjectVisibility::Private),
        "unlisted" => Ok(ObjectVisibility::Unlisted),
        "public" => Ok(ObjectVisibility::Public),
        _ => Err(PortError::Adapter("invalid object visibility".into())),
    }
}

fn parse_state(value: &str) -> Result<GovernedObjectState, PortError> {
    match value {
        "active" => Ok(GovernedObjectState::Active),
        "quarantined" => Ok(GovernedObjectState::Quarantined),
        "tombstoned" => Ok(GovernedObjectState::Tombstoned),
        "erased" => Ok(GovernedObjectState::Erased),
        _ => Err(PortError::Adapter("invalid governed object state".into())),
    }
}

fn parse_malware(value: &str) -> Result<MalwareDisposition, PortError> {
    match value {
        "pending" => Ok(MalwareDisposition::Pending),
        "clean" => Ok(MalwareDisposition::Clean),
        "rejected" => Ok(MalwareDisposition::Rejected),
        _ => Err(PortError::Adapter("invalid malware disposition".into())),
    }
}

fn operation_code(operation: StorageOperation) -> &'static str {
    match operation {
        StorageOperation::Read => "read",
        StorageOperation::ReadRange => "read_range",
        _ => "invalid",
    }
}

const fn stage_code(stage: DeletionStage) -> &'static str {
    match stage {
        DeletionStage::Planned => "planned",
        DeletionStage::Tombstoned => "tombstoned",
        DeletionStage::OriginDeleted => "origin_deleted",
        DeletionStage::CachePurged => "cache_purged",
        DeletionStage::BackupDeleted => "backup_deleted",
        DeletionStage::Verified => "verified",
        DeletionStage::Complete => "complete",
        DeletionStage::Restored => "restored",
    }
}

fn parse_tenant(value: &str) -> Result<TenantId, PortError> {
    TenantId::parse(value).map_err(corrupt)
}

fn parse_timestamp(value: i64) -> Result<TimestampMillis, PortError> {
    TimestampMillis::new(value).map_err(corrupt)
}

fn safe_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or_else(|| PortError::Adapter("unsafe storage authority integer".into()))
}

fn number(value: u64) -> Result<JsValue, PortError> {
    if value > MAX_SAFE_INTEGER {
        return Err(PortError::InvalidRequest("unsafe storage integer".into()));
    }
    Ok(JsValue::from_f64(value as f64))
}

fn signed_number(value: i64) -> Result<JsValue, PortError> {
    number(safe_u64(value)?)
}

fn result_changes(result: &D1Result) -> Result<usize, PortError> {
    if !result.success() {
        return Err(PortError::Adapter(
            "storage authority mutation failed".into(),
        ));
    }
    result
        .meta()
        .map_err(unavailable)?
        .and_then(|meta| meta.changes)
        .ok_or_else(|| PortError::Adapter("storage mutation metadata unavailable".into()))
}

fn unavailable(error: impl std::fmt::Display) -> PortError {
    let _ = error;
    PortError::Adapter("storage authority unavailable".into())
}

fn corrupt(error: impl std::fmt::Display) -> PortError {
    let _ = error;
    PortError::Adapter("storage authority result is invalid".into())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}
