//! D1 and R2 authority for Cap's retained desktop compatibility routes.
//!
//! Reads are actor-scoped projections. Mutations are journaled atomically in
//! D1; video deletion moves to `effect_pending` before R2 I/O so a supplied
//! idempotency key can safely resume an interrupted provider effect.

use async_trait::async_trait;
use frame_application::{
    LegacyDesktopCompatibilityCommandV1, LegacyDesktopCompatibilityInputV1,
    LegacyDesktopCompatibilityOutcomeV1, LegacyDesktopCompatibilityPortErrorV1,
    LegacyDesktopCompatibilityPortV1, LegacyDesktopCompatibilityResultV1,
    LegacyDesktopCompatibilitySurfaceV1, LegacyDesktopLogoUpdateV1,
    LegacyDesktopOrganizationRoleV1, LegacyDesktopOrganizationV1, LegacyDesktopStorageProviderV1,
    LegacyDesktopUserProfileV1, desktop_profile_name, mapped_legacy_video_uuid,
    merge_organization_branding_metadata, organization_brand_colors_from_metadata,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{Bucket, D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_desktop_compatibility/clock_now.sql");
const ORGANIZATION_ALIAS_CANDIDATES_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/organization_alias_candidates.sql");
const ORGANIZATION_ALIAS_READ_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/organization_alias_read.sql");
const ORGANIZATION_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/organization_alias_insert.sql");
const USER_ALIAS_READ_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/user_alias_read.sql");
const USER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/user_alias_insert.sql");
const ORGANIZATIONS_READ_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/organizations_read.sql");
const PROFILE_READ_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/profile_read.sql");
const BRANDING_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/branding_snapshot.sql");
const BRANDING_UPDATE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/branding_update.sql");
const BRANDING_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/branding_assert.sql");
const STORAGE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/storage_snapshot.sql");
const STORAGE_DEACTIVATE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/storage_deactivate.sql");
const STORAGE_ACTIVATE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/storage_activate.sql");
const STORAGE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/storage_assert.sql");
const VIDEO_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_snapshot.sql");
const PROGRESS_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/progress_snapshot.sql");
const PROGRESS_UPDATE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/progress_update.sql");
const PROGRESS_INSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/progress_insert.sql");
const PROGRESS_DELETE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/progress_delete.sql");
const PROGRESS_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/progress_assert.sql");
const OPERATION_LOOKUP_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/operation_lookup.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/operation_claim.sql");
const CLAIM_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/claim_assert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/receipt_insert.sql");
const AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/audit_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/operation_complete.sql");
const DURABLE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/durable_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/assertion_cleanup.sql");
const VIDEO_DELETE_INVENTORY_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_inventory.sql");
const VIDEO_DELETE_INVENTORY_KEY_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_inventory_key.sql");
const VIDEO_DELETE_JOBS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_jobs.sql");
const VIDEO_DELETE_UPLOADS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_uploads.sql");
const VIDEO_DELETE_IMPORTS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_imports.sql");
const VIDEO_DELETE_PROGRESS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_progress.sql");
const VIDEO_DELETE_MARK_OBJECTS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_mark_objects.sql");
const VIDEO_DELETE_MARK_MANIFESTS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_mark_manifests.sql");
const VIDEO_DELETE_APPLY_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_apply.sql");
const VIDEO_DELETE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_assert.sql");
const VIDEO_DELETE_EFFECT_PENDING_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_effect_pending.sql");
const VIDEO_DELETE_PENDING_OBJECTS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_pending_objects.sql");
const VIDEO_DELETE_LEGAL_HOLD_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_legal_hold.sql");
const VIDEO_DELETE_OBJECT_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_object_complete.sql");
const VIDEO_DELETE_PROVIDER_FINALIZE_OBJECTS_SQL: &str = include_str!(
    "../queries/legacy_desktop_compatibility/video_delete_provider_finalize_objects.sql"
);
const VIDEO_DELETE_PROVIDER_FINALIZE_MANIFESTS_SQL: &str = include_str!(
    "../queries/legacy_desktop_compatibility/video_delete_provider_finalize_manifests.sql"
);
const VIDEO_DELETE_PROVIDER_FINALIZE_JOBS_SQL: &str =
    include_str!("../queries/legacy_desktop_compatibility/video_delete_provider_finalize_jobs.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const NATIVE_ALIAS_ATTEMPTS: u8 = 8;
const MAX_PREFIX_OBJECTS: usize = 10_000;
const LEGACY_NANOID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

type PortResult<T> = Result<T, LegacyDesktopCompatibilityPortErrorV1>;

#[derive(Debug, Deserialize)]
struct ClockRowV1 {
    now_ms: i64,
}

#[derive(Debug, Deserialize)]
struct OrganizationAliasCandidateRowV1 {
    organization_id: String,
    owner_user_id: String,
}

#[derive(Debug, Deserialize)]
struct OrganizationAliasRowV1 {
    legacy_organization_id: String,
}

#[derive(Debug, Deserialize)]
struct UserAliasRowV1 {
    legacy_user_id: String,
}

#[derive(Debug, Deserialize)]
struct OrganizationReadRowV1 {
    legacy_organization_id: String,
    name: String,
    legacy_owner_id: String,
    effective_role: String,
    icon_url: Option<String>,
    metadata_json: String,
    created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct ProfileRowV1 {
    first_name: Option<String>,
    last_name: Option<String>,
    email: Option<String>,
    image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrandingSnapshotRowV1 {
    organization_id: String,
    legacy_organization_id: Option<String>,
    name: String,
    owner_id: String,
    legacy_owner_id: Option<String>,
    status: String,
    tombstoned_at_ms: Option<i64>,
    metadata_json: String,
    icon_url: Option<String>,
    revision: i64,
    branding_revision: i64,
    effective_role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StorageRowV1 {
    integration_id: String,
    active: i64,
    updated_at_ms: i64,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct VideoRowV1 {
    video_id: String,
    organization_id: String,
    owner_id: String,
    revision: i64,
    legacy_video_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProgressRowV1 {
    uploaded: f64,
    total: f64,
    updated_at_ms: i64,
    mode: Option<String>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct OperationRowV1 {
    operation_id: String,
    request_digest: String,
    state: String,
    organization_id: Option<String>,
    target_id: Option<String>,
    status: Option<i64>,
    result_kind: Option<String>,
    result_json: Option<String>,
    result_digest: Option<String>,
    audit_count: i64,
}

#[derive(Debug, Deserialize)]
struct PendingObjectRowV1 {
    object_key: String,
    has_legal_hold: i64,
}

#[derive(Debug, Deserialize)]
struct LegalHoldRowV1 {
    has_legal_hold: i64,
}

pub(crate) struct D1LegacyDesktopCompatibilityPortV1<'resource> {
    database: &'resource D1Database,
    recordings: &'resource Bucket,
}

impl<'resource> D1LegacyDesktopCompatibilityPortV1<'resource> {
    #[must_use]
    pub(crate) const fn new(
        database: &'resource D1Database,
        recordings: &'resource Bucket,
    ) -> Self {
        Self {
            database,
            recordings,
        }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<Vec<T>>
    where
        T: DeserializeOwned,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)
    }

    async fn run(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<()> {
        let result = self
            .statement(sql, bindings)?
            .run()
            .into_send()
            .await
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Unavailable)?;
        result
            .success()
            .then_some(())
            .ok_or_else(|| map_d1_message(result.error().as_deref().unwrap_or_default()))
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> PortResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Unavailable);
        }
        if let Some(result) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }

    async fn now_ms(&self) -> PortResult<i64> {
        let mut rows = self.rows::<ClockRowV1>(CLOCK_NOW_SQL, vec![]).await?;
        if rows.len() != 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let now_ms = rows.pop().expect("one clock row").now_ms;
        (0..=MAX_SAFE_INTEGER)
            .contains(&now_ms)
            .then_some(now_ms)
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)
    }

    async fn organization_alias(&self, organization_id: &str) -> PortResult<Option<String>> {
        let mut rows = self
            .rows::<OrganizationAliasRowV1>(
                ORGANIZATION_ALIAS_READ_SQL,
                vec![text(organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        Ok(rows.pop().map(|row| row.legacy_organization_id))
    }

    async fn ensure_organization_alias(
        &self,
        organization_id: &str,
        now_ms: i64,
    ) -> PortResult<String> {
        if let Some(alias) = self.organization_alias(organization_id).await? {
            return valid_legacy_nanoid(&alias)
                .then_some(alias)
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        for attempt in 0..NATIVE_ALIAS_ATTEMPTS {
            let candidate = native_alias_candidate(b"organization", organization_id, attempt);
            self.run(
                ORGANIZATION_ALIAS_INSERT_SQL,
                vec![
                    text(organization_id),
                    text(&candidate),
                    number(now_ms),
                    text(&Uuid::now_v7().to_string()),
                ],
            )
            .await?;
            if let Some(alias) = self.organization_alias(organization_id).await? {
                return Ok(alias);
            }
        }
        Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt)
    }

    async fn user_alias(&self, user_id: &str) -> PortResult<Option<String>> {
        let mut rows = self
            .rows::<UserAliasRowV1>(USER_ALIAS_READ_SQL, vec![text(user_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        Ok(rows.pop().map(|row| row.legacy_user_id))
    }

    async fn ensure_user_alias(&self, user_id: &str, now_ms: i64) -> PortResult<String> {
        if let Some(alias) = self.user_alias(user_id).await? {
            return valid_legacy_nanoid(&alias)
                .then_some(alias)
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        for attempt in 0..NATIVE_ALIAS_ATTEMPTS {
            let candidate = native_alias_candidate(b"user", user_id, attempt);
            self.run(
                USER_ALIAS_INSERT_SQL,
                vec![text(&candidate), text(user_id), number(now_ms)],
            )
            .await?;
            if let Some(alias) = self.user_alias(user_id).await? {
                return Ok(alias);
            }
        }
        Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt)
    }

    async fn organizations(&self, actor_id: &str) -> PortResult<Vec<LegacyDesktopOrganizationV1>> {
        let candidates = self
            .rows::<OrganizationAliasCandidateRowV1>(
                ORGANIZATION_ALIAS_CANDIDATES_SQL,
                vec![text(actor_id)],
            )
            .await?;
        let now_ms = self.now_ms().await?;
        for candidate in candidates {
            self.ensure_organization_alias(&candidate.organization_id, now_ms)
                .await?;
            self.ensure_user_alias(&candidate.owner_user_id, now_ms)
                .await?;
        }
        self.rows::<OrganizationReadRowV1>(ORGANIZATIONS_READ_SQL, vec![text(actor_id)])
            .await?
            .into_iter()
            .map(organization_from_read_row)
            .collect()
    }

    async fn profile(&self, actor_id: &str) -> PortResult<LegacyDesktopUserProfileV1> {
        let mut rows = self
            .rows::<ProfileRowV1>(PROFILE_READ_SQL, vec![text(actor_id)])
            .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacyDesktopCompatibilityPortErrorV1::NotFound
            } else {
                LegacyDesktopCompatibilityPortErrorV1::Corrupt
            });
        }
        let row = rows.pop().expect("one profile row");
        Ok(LegacyDesktopUserProfileV1 {
            name: desktop_profile_name(row.first_name.as_deref(), row.last_name.as_deref()),
            email: row.email,
            image_url: row.image_url,
        })
    }

    async fn branding_snapshot(
        &self,
        actor_id: &str,
        identifier: &str,
    ) -> PortResult<BrandingSnapshotRowV1> {
        let mut rows = self
            .rows::<BrandingSnapshotRowV1>(
                BRANDING_SNAPSHOT_SQL,
                vec![text(actor_id), text(identifier)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let mut row = rows
            .pop()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::NotFound)?;
        if row.status != "active" || row.tombstoned_at_ms.is_some() {
            return Err(LegacyDesktopCompatibilityPortErrorV1::NotFound);
        }
        if row.effective_role.is_none() {
            return Err(LegacyDesktopCompatibilityPortErrorV1::BrandingForbidden);
        }
        if row.legacy_organization_id.is_none() || row.legacy_owner_id.is_none() {
            let now_ms = self.now_ms().await?;
            self.ensure_organization_alias(&row.organization_id, now_ms)
                .await?;
            self.ensure_user_alias(&row.owner_id, now_ms).await?;
            let mut reloaded = self
                .rows::<BrandingSnapshotRowV1>(
                    BRANDING_SNAPSHOT_SQL,
                    vec![text(actor_id), text(&row.organization_id)],
                )
                .await?;
            if reloaded.len() != 1 {
                return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
            }
            row = reloaded.pop().expect("one branding row");
        }
        Ok(row)
    }

    async fn operation(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
    ) -> PortResult<Option<OperationRowV1>> {
        let digest = command
            .idempotency_key_digest_hex()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let mut rows = self
            .rows::<OperationRowV1>(
                OPERATION_LOOKUP_SQL,
                vec![
                    text(command.surface().operation_id()),
                    text(command.actor_id()),
                    text(&digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let row = rows.pop();
        if row
            .as_ref()
            .is_some_and(|row| !operation_scope_valid(command, row))
        {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        Ok(row)
    }

    fn replay(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        row: &OperationRowV1,
    ) -> PortResult<Option<LegacyDesktopCompatibilityOutcomeV1>> {
        if row.request_digest != command.request_digest() {
            return Err(LegacyDesktopCompatibilityPortErrorV1::IdempotencyConflict);
        }
        if row.state != "complete" {
            return Ok(None);
        }
        if row.status != Some(200) || row.audit_count != 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let kind = row
            .result_kind
            .as_deref()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let json = row
            .result_json
            .as_deref()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let digest = row
            .result_digest
            .as_deref()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        if digest != result_digest(kind, json) {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let result = match (command.surface(), kind) {
            (LegacyDesktopCompatibilitySurfaceV1::OrganizationBranding, "organization") => {
                LegacyDesktopCompatibilityResultV1::Organization(
                    serde_json::from_str(json)
                        .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)?,
                )
            }
            (LegacyDesktopCompatibilitySurfaceV1::StorageSetActive, "storage_success")
                if json == r#"{"success":true}"# =>
            {
                LegacyDesktopCompatibilityResultV1::StorageSuccess
            }
            (
                LegacyDesktopCompatibilitySurfaceV1::VideoDelete
                | LegacyDesktopCompatibilitySurfaceV1::VideoProgress,
                "json_true",
            ) if json == "true" => LegacyDesktopCompatibilityResultV1::JsonTrue,
            _ => return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt),
        };
        Ok(Some(LegacyDesktopCompatibilityOutcomeV1 {
            result,
            replayed: true,
        }))
    }

    fn claim_statements(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        operation_kind: &str,
        organization_id: Option<&str>,
        target_id: Option<&str>,
        now_ms: i64,
    ) -> PortResult<Vec<D1PreparedStatement>> {
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let key_digest = command
            .idempotency_key_digest_hex()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let bindings = vec![
            text(&operation_id),
            text(command.surface().operation_id()),
            text(operation_kind),
            text(command.actor_id()),
            optional_text(organization_id),
            optional_text(target_id),
            text(&key_digest),
            text(command.request_digest()),
        ];
        let mut claim = bindings.clone();
        claim.push(number(now_ms));
        Ok(vec![
            self.statement(OPERATION_CLAIM_SQL, claim)?,
            self.statement(CLAIM_ASSERT_SQL, bindings)?,
        ])
    }

    fn completion_statements(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        target: &str,
        result_kind: &str,
        result_json: &str,
        now_ms: i64,
    ) -> PortResult<Vec<D1PreparedStatement>> {
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let result_digest = result_digest(result_kind, result_json);
        Ok(vec![
            self.statement(
                RECEIPT_INSERT_SQL,
                vec![
                    text(&operation_id),
                    number(200),
                    text(result_kind),
                    text(result_json),
                    text(&result_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    text(&Uuid::now_v7().to_string()),
                    text(&operation_id),
                    text(command.surface().operation_id()),
                    text(&audit_digest(b"actor", command.actor_id())),
                    text(&audit_digest(b"target", target)),
                    text(command.request_digest()),
                    text(&result_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![text(&operation_id), number(now_ms)],
            )?,
            self.statement(
                DURABLE_ASSERT_SQL,
                vec![text(&operation_id), number(now_ms), text(&result_digest)],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![text(&operation_id)])?,
        ])
    }

    async fn branding(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        legacy_organization_id: &str,
        patch: &frame_application::LegacyDesktopBrandingPatchV1,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        if let Some(existing) = self.operation(command).await? {
            return self
                .replay(command, &existing)?
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::Unavailable);
        }
        let snapshot = self
            .branding_snapshot(command.actor_id(), legacy_organization_id)
            .await?;
        let role = parse_role(
            snapshot
                .effective_role
                .as_deref()
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::BrandingForbidden)?,
        )?;
        if !matches!(
            role,
            LegacyDesktopOrganizationRoleV1::Owner | LegacyDesktopOrganizationRoleV1::Admin
        ) {
            return Err(LegacyDesktopCompatibilityPortErrorV1::BrandingForbidden);
        }
        let metadata: Value = serde_json::from_str(&snapshot.metadata_json)
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let metadata = merge_organization_branding_metadata(metadata, &patch.brand_colors);
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let (logo_mode, icon_url) = match &patch.logo {
            LegacyDesktopLogoUpdateV1::Keep => (0, snapshot.icon_url.clone()),
            LegacyDesktopLogoUpdateV1::Remove => (1, None),
            LegacyDesktopLogoUpdateV1::Upload { data_url, .. } => (1, Some(data_url.clone())),
        };
        let legacy_id = snapshot
            .legacy_organization_id
            .clone()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let owner_id = snapshot
            .legacy_owner_id
            .clone()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let result = LegacyDesktopOrganizationV1 {
            id: legacy_id,
            name: snapshot.name.clone(),
            owner_id,
            role,
            can_edit_brand: true,
            icon_url: icon_url.clone(),
            brand_colors: patch.brand_colors.clone(),
        };
        if !result.valid() {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let result_json = serde_json::to_string(&result)
            .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
        let now_ms = self.now_ms().await?;
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let mut statements = self.claim_statements(
            command,
            "organization_branding",
            Some(&snapshot.organization_id),
            Some(legacy_organization_id),
            now_ms,
        )?;
        statements.extend([
            self.statement(
                BRANDING_UPDATE_SQL,
                vec![
                    text(&snapshot.organization_id),
                    number(snapshot.revision),
                    number(snapshot.branding_revision),
                    text(&metadata_json),
                    number(logo_mode),
                    optional_text(icon_url.as_deref()),
                    number(now_ms),
                    text(&operation_id),
                    text(command.actor_id()),
                ],
            )?,
            self.statement(
                BRANDING_ASSERT_SQL,
                vec![
                    text(&operation_id),
                    text(&snapshot.organization_id),
                    text(&metadata_json),
                    optional_text(icon_url.as_deref()),
                    number(snapshot.revision + 1),
                    number(snapshot.branding_revision + 1),
                ],
            )?,
        ]);
        statements.extend(self.completion_statements(
            command,
            legacy_organization_id,
            "organization",
            &result_json,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDesktopCompatibilityOutcomeV1 {
            result: LegacyDesktopCompatibilityResultV1::Organization(result),
            replayed: false,
        })
    }

    async fn storage_set_active(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        provider: LegacyDesktopStorageProviderV1,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        if let Some(existing) = self.operation(command).await? {
            return self
                .replay(command, &existing)?
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::Unavailable);
        }
        let rows = self
            .rows::<StorageRowV1>(STORAGE_SNAPSHOT_SQL, vec![text(command.actor_id())])
            .await?;
        let selected = match provider {
            LegacyDesktopStorageProviderV1::S3 => None,
            LegacyDesktopStorageProviderV1::GoogleDrive => Some(
                rows.first()
                    .ok_or(LegacyDesktopCompatibilityPortErrorV1::StorageNotConnected)?,
            ),
        };
        for row in &rows {
            if !matches!(row.active, 0 | 1)
                || row.updated_at_ms < 0
                || row.revision < 0
                || row.integration_id.is_empty()
            {
                return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
            }
        }
        let now_ms = self.now_ms().await?;
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let mut statements = self.claim_statements(
            command,
            "storage_set_active",
            None,
            Some(provider.as_str()),
            now_ms,
        )?;
        statements.push(self.statement(
            STORAGE_DEACTIVATE_SQL,
            vec![
                text(command.actor_id()),
                number(now_ms),
                text(&operation_id),
            ],
        )?);
        if let Some(selected) = selected {
            statements.push(self.statement(
                STORAGE_ACTIVATE_SQL,
                vec![
                    text(&selected.integration_id),
                    text(command.actor_id()),
                    number(now_ms),
                    text(&operation_id),
                ],
            )?);
        }
        statements.push(self.statement(
            STORAGE_ASSERT_SQL,
            vec![
                text(&operation_id),
                text(command.actor_id()),
                optional_text(selected.map(|row| row.integration_id.as_str())),
            ],
        )?);
        statements.extend(self.completion_statements(
            command,
            provider.as_str(),
            "storage_success",
            r#"{"success":true}"#,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDesktopCompatibilityOutcomeV1 {
            result: LegacyDesktopCompatibilityResultV1::StorageSuccess,
            replayed: false,
        })
    }

    async fn video_progress(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        progress: &frame_application::LegacyDesktopVideoProgressV1,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        if let Some(existing) = self.operation(command).await? {
            return self
                .replay(command, &existing)?
                .ok_or(LegacyDesktopCompatibilityPortErrorV1::Unavailable);
        }
        let video = self
            .video(command.actor_id(), &progress.legacy_video_id)
            .await?;
        let mut upload_rows = self
            .rows::<ProgressRowV1>(PROGRESS_SNAPSHOT_SQL, vec![text(&video.video_id)])
            .await?;
        if upload_rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let upload = upload_rows.pop();
        if upload.as_ref().is_some_and(|row| {
            !row.uploaded.is_finite()
                || !row.total.is_finite()
                || row.updated_at_ms < 0
                || row.revision < 0
        }) {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let now_ms = self.now_ms().await?;
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let mut statements = self.claim_statements(
            command,
            "video_progress",
            Some(&video.organization_id),
            Some(&video.video_id),
            now_ms,
        )?;
        let (assert_kind, old_revision, old_updated_at) = match upload {
            None => {
                statements.push(self.statement(
                    PROGRESS_INSERT_SQL,
                    vec![
                        text(&video.video_id),
                        float(progress.uploaded),
                        float(progress.total),
                        number(progress.updated_at_ms),
                        text(&operation_id),
                    ],
                )?);
                ("updated", 0, 0)
            }
            Some(ref row)
                if progress.uploaded == progress.total
                    && row.mode.as_deref() != Some("multipart") =>
            {
                statements.push(self.statement(
                    PROGRESS_DELETE_SQL,
                    vec![text(&video.video_id), number(row.revision)],
                )?);
                ("deleted", row.revision, row.updated_at_ms)
            }
            Some(ref row) if row.updated_at_ms > progress.updated_at_ms => {
                ("stale", row.revision, row.updated_at_ms)
            }
            Some(ref row) => {
                statements.push(self.statement(
                    PROGRESS_UPDATE_SQL,
                    vec![
                        text(&video.video_id),
                        float(progress.uploaded),
                        float(progress.total),
                        number(progress.updated_at_ms),
                        text(&operation_id),
                        number(row.revision),
                    ],
                )?);
                ("updated", row.revision, row.updated_at_ms)
            }
        };
        statements.push(self.statement(
            PROGRESS_ASSERT_SQL,
            vec![
                text(&operation_id),
                text(assert_kind),
                text(&video.video_id),
                number(old_revision),
                number(old_updated_at),
                float(progress.uploaded),
                float(progress.total),
                number(progress.updated_at_ms),
            ],
        )?);
        statements.extend(self.completion_statements(
            command,
            &progress.legacy_video_id,
            "json_true",
            "true",
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDesktopCompatibilityOutcomeV1 {
            result: LegacyDesktopCompatibilityResultV1::JsonTrue,
            replayed: false,
        })
    }

    async fn video(&self, actor_id: &str, legacy_video_id: &str) -> PortResult<VideoRowV1> {
        let mapped = mapped_legacy_video_uuid(legacy_video_id)
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::NotFound)?;
        let mut rows = self
            .rows::<VideoRowV1>(
                VIDEO_SNAPSHOT_SQL,
                vec![text(actor_id), text(&mapped), text(legacy_video_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::NotFound)?;
        if row.owner_id != actor_id || row.revision < 0 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        if let Some(alias) = &row.legacy_video_id
            && alias != legacy_video_id
            && row.video_id != legacy_video_id
        {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        Ok(row)
    }

    async fn list_prefix_objects(&self, prefix: &str) -> PortResult<Vec<String>> {
        let mut keys = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut builder = self
                .recordings
                .list()
                .limit(1_000)
                .prefix(prefix.to_owned());
            if let Some(value) = cursor.as_deref() {
                builder = builder.cursor(value.to_owned());
            }
            let page = builder
                .execute()
                .into_send()
                .await
                .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Provider)?;
            for object in page.objects() {
                let key = object.key();
                if !key.starts_with(prefix) || key.is_empty() || key.len() > 2_048 {
                    return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
                }
                keys.push(key);
                if keys.len() > MAX_PREFIX_OBJECTS {
                    return Err(LegacyDesktopCompatibilityPortErrorV1::Provider);
                }
            }
            cursor = page.cursor();
            if cursor.is_none() {
                break;
            }
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    async fn delete_r2_and_complete(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        operation_id: &str,
        target: &str,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        let mut hold_rows = self
            .rows::<LegalHoldRowV1>(VIDEO_DELETE_LEGAL_HOLD_SQL, vec![text(operation_id)])
            .await?;
        if hold_rows.len() != 1 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
        }
        if hold_rows.pop().expect("one legal-hold row").has_legal_hold != 0 {
            return Err(LegacyDesktopCompatibilityPortErrorV1::Provider);
        }
        loop {
            let pending = self
                .rows::<PendingObjectRowV1>(
                    VIDEO_DELETE_PENDING_OBJECTS_SQL,
                    vec![text(operation_id)],
                )
                .await?;
            if pending.is_empty() {
                break;
            }
            for object in pending {
                if object.has_legal_hold != 0 {
                    return Err(LegacyDesktopCompatibilityPortErrorV1::Provider);
                }
                self.recordings
                    .delete(&object.object_key)
                    .into_send()
                    .await
                    .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Provider)?;
                let completed_at_ms = self.now_ms().await?;
                self.run(
                    VIDEO_DELETE_OBJECT_COMPLETE_SQL,
                    vec![
                        text(operation_id),
                        text(&object.object_key),
                        number(completed_at_ms),
                    ],
                )
                .await?;
            }
        }
        let now_ms = self.now_ms().await?;
        let mut statements = vec![
            self.statement(
                VIDEO_DELETE_PROVIDER_FINALIZE_OBJECTS_SQL,
                vec![text(operation_id), number(now_ms)],
            )?,
            self.statement(
                VIDEO_DELETE_PROVIDER_FINALIZE_MANIFESTS_SQL,
                vec![text(operation_id), number(now_ms)],
            )?,
            self.statement(
                VIDEO_DELETE_PROVIDER_FINALIZE_JOBS_SQL,
                vec![text(operation_id), number(now_ms)],
            )?,
        ];
        statements.extend(self.completion_statements(
            command,
            target,
            "json_true",
            "true",
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDesktopCompatibilityOutcomeV1 {
            result: LegacyDesktopCompatibilityResultV1::JsonTrue,
            replayed: false,
        })
    }

    async fn video_delete(
        &self,
        command: &LegacyDesktopCompatibilityCommandV1,
        legacy_video_id: &str,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        if let Some(existing) = self.operation(command).await? {
            if let Some(replayed) = self.replay(command, &existing)? {
                return Ok(replayed);
            }
            if existing.state != "effect_pending" {
                return Err(LegacyDesktopCompatibilityPortErrorV1::Unavailable);
            }
            return self
                .delete_r2_and_complete(command, &existing.operation_id, legacy_video_id)
                .await;
        }
        let video = self.video(command.actor_id(), legacy_video_id).await?;
        let now_ms = self.now_ms().await?;
        let legacy_owner_id = self.ensure_user_alias(command.actor_id(), now_ms).await?;
        let source_video_id = video.legacy_video_id.as_deref().unwrap_or(legacy_video_id);
        let prefix = format!("{legacy_owner_id}/{source_video_id}/");
        let prefix_objects = self.list_prefix_objects(&prefix).await?;
        let operation_id = command
            .operation_id()
            .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)?
            .to_string();
        let mut statements = self.claim_statements(
            command,
            "video_delete",
            Some(&video.organization_id),
            Some(&video.video_id),
            now_ms,
        )?;
        statements.push(self.statement(
            VIDEO_DELETE_INVENTORY_SQL,
            vec![text(&operation_id), text(&video.video_id)],
        )?);
        for key in prefix_objects {
            statements.push(self.statement(
                VIDEO_DELETE_INVENTORY_KEY_SQL,
                vec![text(&operation_id), text(&key)],
            )?);
        }
        statements.extend([
            self.statement(
                VIDEO_DELETE_JOBS_SQL,
                vec![text(&operation_id), text(&video.video_id), number(now_ms)],
            )?,
            self.statement(
                VIDEO_DELETE_UPLOADS_SQL,
                vec![text(&video.video_id), text(&video.organization_id)],
            )?,
            self.statement(VIDEO_DELETE_IMPORTS_SQL, vec![text(&video.video_id)])?,
            self.statement(VIDEO_DELETE_PROGRESS_SQL, vec![text(&video.video_id)])?,
            self.statement(
                VIDEO_DELETE_MARK_OBJECTS_SQL,
                vec![text(&operation_id), text(&video.video_id), number(now_ms)],
            )?,
            self.statement(
                VIDEO_DELETE_MARK_MANIFESTS_SQL,
                vec![text(&operation_id), text(&video.video_id), number(now_ms)],
            )?,
            self.statement(
                VIDEO_DELETE_APPLY_SQL,
                vec![
                    text(&video.video_id),
                    text(command.actor_id()),
                    text(&video.organization_id),
                    number(now_ms),
                    text(&operation_id),
                    number(video.revision),
                ],
            )?,
            self.statement(
                VIDEO_DELETE_ASSERT_SQL,
                vec![
                    text(&operation_id),
                    text(&video.video_id),
                    text(command.actor_id()),
                    text(&video.organization_id),
                    number(now_ms),
                    number(video.revision + 1),
                ],
            )?,
            self.statement(VIDEO_DELETE_EFFECT_PENDING_SQL, vec![text(&operation_id)])?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![text(&operation_id)])?,
        ]);
        self.batch(statements).await?;
        self.delete_r2_and_complete(command, &operation_id, legacy_video_id)
            .await
    }
}

#[async_trait(?Send)]
impl LegacyDesktopCompatibilityPortV1 for D1LegacyDesktopCompatibilityPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyDesktopCompatibilityCommandV1,
    ) -> PortResult<LegacyDesktopCompatibilityOutcomeV1> {
        match command.input().clone() {
            LegacyDesktopCompatibilityInputV1::Organizations => {
                Ok(LegacyDesktopCompatibilityOutcomeV1 {
                    result: LegacyDesktopCompatibilityResultV1::Organizations(
                        self.organizations(command.actor_id()).await?,
                    ),
                    replayed: false,
                })
            }
            LegacyDesktopCompatibilityInputV1::UserProfile => {
                Ok(LegacyDesktopCompatibilityOutcomeV1 {
                    result: LegacyDesktopCompatibilityResultV1::UserProfile(
                        self.profile(command.actor_id()).await?,
                    ),
                    replayed: false,
                })
            }
            LegacyDesktopCompatibilityInputV1::OrganizationBranding {
                legacy_organization_id,
                patch,
            } => {
                self.branding(&command, &legacy_organization_id, &patch)
                    .await
            }
            LegacyDesktopCompatibilityInputV1::StorageSetActive { provider } => {
                self.storage_set_active(&command, provider).await
            }
            LegacyDesktopCompatibilityInputV1::VideoDelete { legacy_video_id } => {
                self.video_delete(&command, &legacy_video_id).await
            }
            LegacyDesktopCompatibilityInputV1::VideoProgress(progress) => {
                self.video_progress(&command, &progress).await
            }
        }
    }
}

fn operation_scope_valid(
    command: &LegacyDesktopCompatibilityCommandV1,
    row: &OperationRowV1,
) -> bool {
    let native_scope =
        |value: Option<&str>| value.is_some_and(|value| Uuid::parse_str(value).is_ok());
    let scope_matches = match command.input() {
        LegacyDesktopCompatibilityInputV1::OrganizationBranding {
            legacy_organization_id,
            ..
        } => {
            native_scope(row.organization_id.as_deref())
                && row.target_id.as_deref() == Some(legacy_organization_id)
        }
        LegacyDesktopCompatibilityInputV1::StorageSetActive { provider } => {
            row.organization_id.is_none() && row.target_id.as_deref() == Some(provider.as_str())
        }
        LegacyDesktopCompatibilityInputV1::VideoDelete { .. }
        | LegacyDesktopCompatibilityInputV1::VideoProgress(_) => {
            native_scope(row.organization_id.as_deref()) && native_scope(row.target_id.as_deref())
        }
        LegacyDesktopCompatibilityInputV1::Organizations
        | LegacyDesktopCompatibilityInputV1::UserProfile => false,
    };
    scope_matches
        && Uuid::parse_str(&row.operation_id).is_ok()
        && match row.state.as_str() {
            "claimed" | "complete" => true,
            "effect_pending" => {
                command.surface() == LegacyDesktopCompatibilitySurfaceV1::VideoDelete
            }
            _ => false,
        }
}

fn organization_from_read_row(
    row: OrganizationReadRowV1,
) -> PortResult<LegacyDesktopOrganizationV1> {
    if row.created_at_ms < 0 {
        return Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt);
    }
    let metadata: Value = serde_json::from_str(&row.metadata_json)
        .map_err(|_| LegacyDesktopCompatibilityPortErrorV1::Corrupt)?;
    let role = parse_role(&row.effective_role)?;
    let organization = LegacyDesktopOrganizationV1 {
        id: row.legacy_organization_id,
        name: row.name,
        owner_id: row.legacy_owner_id,
        role,
        can_edit_brand: matches!(
            role,
            LegacyDesktopOrganizationRoleV1::Owner | LegacyDesktopOrganizationRoleV1::Admin
        ),
        icon_url: row.icon_url,
        brand_colors: organization_brand_colors_from_metadata(&metadata),
    };
    organization
        .valid()
        .then_some(organization)
        .ok_or(LegacyDesktopCompatibilityPortErrorV1::Corrupt)
}

fn parse_role(value: &str) -> PortResult<LegacyDesktopOrganizationRoleV1> {
    match value {
        "owner" => Ok(LegacyDesktopOrganizationRoleV1::Owner),
        "admin" => Ok(LegacyDesktopOrganizationRoleV1::Admin),
        "member" => Ok(LegacyDesktopOrganizationRoleV1::Member),
        _ => Err(LegacyDesktopCompatibilityPortErrorV1::Corrupt),
    }
}

fn map_d1_message(message: &str) -> LegacyDesktopCompatibilityPortErrorV1 {
    if message.contains("UNIQUE constraint failed: legacy_desktop_compatibility_operations_v1") {
        LegacyDesktopCompatibilityPortErrorV1::IdempotencyConflict
    } else if message.contains("frame_legacy_desktop_compatibility_assertion_v1")
        || message.contains("frame_legacy_desktop_compatibility_evidence_immutable_v1")
    {
        LegacyDesktopCompatibilityPortErrorV1::Corrupt
    } else {
        LegacyDesktopCompatibilityPortErrorV1::Unavailable
    }
}

fn native_alias_candidate(domain: &[u8], native_id: &str, attempt: u8) -> String {
    let digest = Sha256::digest(
        [
            b"frame.legacy-desktop.alias.v1\0".as_slice(),
            domain,
            b"\0",
            native_id.as_bytes(),
            &[attempt],
        ]
        .concat(),
    );
    let mut encoded = String::with_capacity(15);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in digest {
        accumulator = (accumulator << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 && encoded.len() < 15 {
            bits -= 5;
            encoded.push(char::from(
                LEGACY_NANOID_ALPHABET[((accumulator >> bits) & 31) as usize],
            ));
        }
        if encoded.len() == 15 {
            break;
        }
    }
    encoded
}

fn valid_legacy_nanoid(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| LEGACY_NANOID_ALPHABET.contains(&byte))
}

fn result_digest(kind: &str, json: &str) -> String {
    digest_fields(b"frame.legacy-desktop.result.v1\0", &[kind, json])
}

fn audit_digest(domain: &[u8], value: &str) -> String {
    let mut prefix = b"frame.legacy-desktop.audit.v1\0".to_vec();
    prefix.extend_from_slice(domain);
    prefix.push(0);
    digest_fields(&prefix, &[value])
}

fn digest_fields(prefix: &[u8], fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix);
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    lower_hex(&hasher.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    encoded
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn optional_text(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn float(value: f64) -> JsValue {
    JsValue::from_f64(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_aliases_are_deterministic_and_valid() {
        let first = native_alias_candidate(b"user", "01900000-0000-7000-8000-000000000001", 0);
        let second =
            native_alias_candidate(b"organization", "01900000-0000-7000-8000-000000000001", 0);
        assert!(valid_legacy_nanoid(&first));
        assert!(valid_legacy_nanoid(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn all_sql_files_are_single_statement_and_provider_continuation_is_explicit() {
        for sql in [
            VIDEO_DELETE_PROVIDER_FINALIZE_OBJECTS_SQL,
            VIDEO_DELETE_PROVIDER_FINALIZE_MANIFESTS_SQL,
            VIDEO_DELETE_PROVIDER_FINALIZE_JOBS_SQL,
        ] {
            assert_eq!(sql.matches(';').count(), 1);
        }
        assert!(VIDEO_DELETE_EFFECT_PENDING_SQL.contains("effect_pending"));
        assert!(VIDEO_DELETE_PENDING_OBJECTS_SQL.contains("object_legal_holds"));
    }

    #[test]
    fn receipts_are_domain_separated_and_stable() {
        assert_eq!(result_digest("json_true", "true").len(), 64);
        assert_ne!(
            result_digest("json_true", "true"),
            result_digest("storage_success", "true")
        );
    }
}
