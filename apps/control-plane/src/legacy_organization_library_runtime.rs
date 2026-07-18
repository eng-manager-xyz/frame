//! D1/R2 authority for the 21 provider-free organization/library actions.

use std::collections::HashMap;

use async_trait::async_trait;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use frame_application::{
    LegacyCollectionKindV1, LegacyImagePayloadV1, LegacyOrganizationLibraryActionV1,
    LegacyOrganizationLibraryAtomicErrorV1, LegacyOrganizationLibraryAtomicOutcomeV1,
    LegacyOrganizationLibraryAtomicPortV1, LegacyOrganizationLibraryBrowserFenceV1,
    LegacyOrganizationLibraryCommandV1, LegacyOrganizationLibraryEffectsV1,
    LegacyOrganizationLibraryInputV1, LegacyOrganizationLibraryReceiptV1,
    LegacyOrganizationLibraryResultV1, LegacyOrganizationStorageProviderV1,
};
use frame_domain::LegacyCapNanoId;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{Bucket, D1Database, D1PreparedStatement, HttpMetadata, send::IntoSendFuture};

const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_organization_library/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_organization_library/operation_claim.sql");
const OPERATION_SET_PENDING_SQL: &str =
    include_str!("../queries/legacy_organization_library/operation_set_pending.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_organization_library/operation_complete.sql");
const R2_EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/r2_effect_insert.sql");
const R2_EFFECTS_PENDING_SQL: &str =
    include_str!("../queries/legacy_organization_library/r2_effects_pending.sql");
const R2_EFFECT_APPLIED_SQL: &str =
    include_str!("../queries/legacy_organization_library/r2_effect_applied.sql");
const BROWSER_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/browser_grant_assert.sql");
const BROWSER_GRANT_DELETE_SQL: &str =
    include_str!("../queries/legacy_organization_library/browser_grant_delete.sql");
const CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/changes_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_organization_library/assertion_cleanup.sql");
const AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/audit_insert.sql");
const ORGANIZATION_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/organization_authority_assert.sql");
const TARGET_MANAGER_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/target_manager_assert.sql");
const ACTOR_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/actor_assert.sql");
const SPACE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/space_authority_assert.sql");
const FOLDER_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/folder_authority_assert.sql");
const MEMBER_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/member_authority_assert.sql");
const ACTIVE_ORGANIZATION_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/active_organization_snapshot.sql");
const TARGET_ORGANIZATION_MANAGER_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/target_organization_manager_snapshot.sql");
const ACTOR_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/actor_snapshot.sql");
const SPACE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/space_snapshot.sql");
const FOLDER_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/folder_snapshot.sql");
const MEMBER_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/member_snapshot.sql");
const PASSWORD_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/password_snapshot.sql");
const STORAGE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_organization_library/storage_snapshot.sql");
const PRO_SEAT_COUNT_SQL: &str =
    include_str!("../queries/legacy_organization_library/pro_seat_count.sql");
const REMAINING_PRO_SUBSCRIPTION_SQL: &str =
    include_str!("../queries/legacy_organization_library/remaining_pro_subscription.sql");
const SET_SPACE_VISIBILITY_SQL: &str =
    include_str!("../queries/legacy_organization_library/set_space_visibility.sql");
const SET_SPACE_LOGO_SQL: &str =
    include_str!("../queries/legacy_organization_library/set_space_logo.sql");
const SET_FOLDER_LOGO_SQL: &str =
    include_str!("../queries/legacy_organization_library/set_folder_logo.sql");
const DELETE_SPACE_SQL: &str =
    include_str!("../queries/legacy_organization_library/delete_space.sql");
const REMOVE_MEMBER_SPACE_MEMBERSHIPS_SQL: &str =
    include_str!("../queries/legacy_organization_library/remove_member_space_memberships.sql");
const REMOVE_MEMBER_INVITES_SQL: &str =
    include_str!("../queries/legacy_organization_library/remove_member_invites.sql");
const REMOVE_MEMBER_ALIAS_SQL: &str =
    include_str!("../queries/legacy_organization_library/remove_member_alias.sql");
const REMOVE_MEMBER_SQL: &str =
    include_str!("../queries/legacy_organization_library/remove_member.sql");
const UPDATE_ORGANIZATION_SETTINGS_SQL: &str =
    include_str!("../queries/legacy_organization_library/update_organization_settings.sql");
const PATCH_ORGANIZATION_BRANDING_SQL: &str =
    include_str!("../queries/legacy_organization_library/patch_organization_branding.sql");
const SELECT_ACTIVE_ORGANIZATION_SQL: &str =
    include_str!("../queries/legacy_organization_library/select_active_organization.sql");
const DISCONNECT_GOOGLE_DRIVE_SQL: &str =
    include_str!("../queries/legacy_organization_library/disconnect_google_drive.sql");
const DISABLE_STORAGE_INTEGRATIONS_SQL: &str =
    include_str!("../queries/legacy_organization_library/disable_storage_integrations.sql");
const ENABLE_STORAGE_INTEGRATION_SQL: &str =
    include_str!("../queries/legacy_organization_library/enable_storage_integration.sql");
const TOGGLE_PRO_SEAT_SQL: &str =
    include_str!("../queries/legacy_organization_library/toggle_pro_seat.sql");
const UPDATE_MEMBER_SUBSCRIPTION_LINK_SQL: &str =
    include_str!("../queries/legacy_organization_library/update_member_subscription_link.sql");
const UPDATE_ORGANIZATION_DETAILS_SQL: &str =
    include_str!("../queries/legacy_organization_library/update_organization_details.sql");
const UPDATE_MEMBER_ROLE_SQL: &str =
    include_str!("../queries/legacy_organization_library/update_member_role.sql");
const UPLOAD_SPACE_ICON_SQL: &str =
    include_str!("../queries/legacy_organization_library/upload_space_icon.sql");
const CREATE_ORGANIZATION_SQL: &str =
    include_str!("../queries/legacy_organization_library/create_organization.sql");
const CREATE_ORGANIZATION_MEMBER_SQL: &str =
    include_str!("../queries/legacy_organization_library/create_organization_member.sql");
const CREATE_ORGANIZATION_ALIAS_SQL: &str =
    include_str!("../queries/legacy_organization_library/create_organization_alias.sql");
const ORGANIZATION_NAME_ASSERT_SQL: &str =
    include_str!("../queries/legacy_organization_library/organization_name_assert.sql");

const MAX_R2_PREFIX_PAGES: usize = 10;
const MAX_R2_PREFIX_OBJECTS: usize = 10_000;

type AtomicResult<T> = Result<T, LegacyOrganizationLibraryAtomicErrorV1>;

#[derive(Clone)]
pub(crate) struct LegacyOrganizationLibraryLocalConfigV1 {
    pub(crate) google_client_id: String,
    pub(crate) google_redirect_uri: String,
    pub(crate) google_auth_base_url: String,
    pub(crate) state_secret: Vec<u8>,
    pub(crate) google_picker_api_key: Option<String>,
}

pub(crate) struct D1R2LegacyOrganizationLibraryPortV1<'a> {
    database: &'a D1Database,
    bucket: &'a Bucket,
    config: &'a LegacyOrganizationLibraryLocalConfigV1,
    now_ms: i64,
}

impl<'a> D1R2LegacyOrganizationLibraryPortV1<'a> {
    #[must_use]
    pub(crate) const fn new(
        database: &'a D1Database,
        bucket: &'a Bucket,
        config: &'a LegacyOrganizationLibraryLocalConfigV1,
        now_ms: i64,
    ) -> Self {
        Self {
            database,
            bucket,
            config,
            now_ms,
        }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    organization_id: Option<String>,
    actor_id: String,
    action: String,
    request_digest: String,
    state: String,
    result_json: Option<String>,
    effects_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ActiveOrganizationRow {
    organization_id: String,
    owner_id: String,
    name: String,
    settings_json: String,
    legacy_icon_key: Option<String>,
    legacy_shareable_link_icon_key: Option<String>,
    legacy_workos_organization_id: Option<String>,
    legacy_workos_connection_id: Option<String>,
    organization_revision: i64,
    organization_authority_version: i64,
    legacy_organization_library_revision: i64,
    organization_preference_revision: i64,
    legacy_stripe_subscription_status: Option<String>,
    legacy_third_party_stripe_subscription_id: Option<String>,
    actor_role: Option<String>,
    actor_membership_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TargetManagerRow {
    organization_id: String,
    organization_revision: i64,
    organization_authority_version: i64,
    organization_preference_revision: i64,
}

#[derive(Debug, Deserialize)]
struct ActorRow {
    id: String,
    organization_preference_revision: i64,
}

#[derive(Debug, Deserialize)]
struct SpaceRow {
    space_id: String,
    organization_id: String,
    is_public: i64,
    settings_json: String,
    legacy_icon_key: Option<String>,
    space_revision: i64,
    space_authority_version: i64,
    legacy_organization_library_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    owner_subscription_status: Option<String>,
    owner_third_party_subscription_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FolderRow {
    folder_id: String,
    organization_id: String,
    settings_json: String,
    folder_revision: i64,
    tree_revision: i64,
    legacy_organization_library_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    owner_subscription_status: Option<String>,
    owner_third_party_subscription_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemberRow {
    legacy_member_id: String,
    organization_id: String,
    target_user_id: String,
    target_role: String,
    target_state: String,
    has_pro_seat: i64,
    target_revision: i64,
    target_authority_version: i64,
    owner_id: String,
    actor_role: Option<String>,
    actor_state: Option<String>,
    target_email: String,
    owner_invite_quota: i64,
    owner_subscription_id: Option<String>,
    owner_subscription_status: Option<String>,
    actor_invite_quota: i64,
    actor_subscription_id: Option<String>,
    actor_subscription_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PasswordRow {
    collection_id: String,
    organization_id: String,
    password_hash: Option<String>,
    password_revision: i64,
    collection_kind: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct StorageRow {
    id: String,
    provider: String,
    state: String,
    capabilities_json: String,
    revision: i64,
    authority_version: i64,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    assigned_count: i64,
}

#[derive(Debug, Deserialize)]
struct R2EffectRow {
    effect_order: i64,
    effect_kind: String,
    object_key: String,
    checksum_sha256: Option<String>,
    content_type: Option<String>,
}

enum AuthorityFence {
    Organization {
        row: ActiveOrganizationRow,
        mode: &'static str,
    },
    TargetManager(TargetManagerRow),
    Actor(ActorRow),
    Space(SpaceRow),
    Folder(FolderRow),
    Member(MemberRow),
}

struct PutEffect {
    object_key: String,
    image: LegacyImagePayloadV1,
}

struct DeleteEffect {
    kind: &'static str,
    object_key: String,
}

struct R2EffectInsert<'a> {
    operation_id: &'a str,
    order: i64,
    kind: &'a str,
    object_key: &'a str,
    checksum: Option<&'a str>,
    content_type: Option<&'a str>,
    state: &'a str,
    applied_at_ms: Option<i64>,
}

struct MutationStatement {
    sql: &'static str,
    bindings: Vec<JsValue>,
    expected_changes: Option<(String, i64)>,
}

struct ExecutionPlan {
    organization_id: String,
    authority: AuthorityFence,
    result: LegacyOrganizationLibraryResultV1,
    effects: LegacyOrganizationLibraryEffectsV1,
    mutations: Vec<MutationStatement>,
    puts: Vec<PutEffect>,
    deletes: Vec<DeleteEffect>,
}

#[async_trait]
impl LegacyOrganizationLibraryAtomicPortV1 for D1R2LegacyOrganizationLibraryPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyOrganizationLibraryCommandV1,
        browser_fence: Option<&LegacyOrganizationLibraryBrowserFenceV1>,
    ) -> AtomicResult<LegacyOrganizationLibraryAtomicOutcomeV1> {
        if command.action() == LegacyOrganizationLibraryActionV1::VerifyCollectionPassword {
            if browser_fence.is_some() {
                return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
            }
            return self.verify_collection_password(command).await;
        }
        let fence = browser_fence.ok_or(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied)?;
        if command.actor_id() != Some(fence.actor_id())
            || command.idempotency_key().is_none()
            || !valid_safe_integer(self.now_ms)
        {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }

        let actor_id = command
            .actor_id()
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?
            .to_string();
        let action = action_name(command.action());
        let key_digest = digest_text(
            command
                .idempotency_key()
                .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?
                .expose(),
        );
        let mut existing = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(action),
                    JsValue::from_str(&key_digest),
                ],
            )
            .await?;
        if existing.len() > 1 {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        if let Some(operation) = existing.pop() {
            return self.resume(command, &operation).await;
        }

        let operation_id = Uuid::new_v4().to_string();
        let mut plan = self.prepare_plan(command, &operation_id).await?;
        for put in &plan.puts {
            self.apply_put(put).await?;
        }
        let receipt = LegacyOrganizationLibraryReceiptV1 {
            action: command.action(),
            request_digest: *command.request_digest(),
            result: plan.result.clone(),
            effects: plan.effects.clone(),
        };
        if !receipt.matches(command) {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        let result_json = serde_json::to_string(&receipt.result)
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
        let effects_json = serde_json::to_string(&receipt.effects)
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;

        let mut statements = Vec::new();
        statements.push(self.statement(
            OPERATION_CLAIM_SQL,
            vec![
                JsValue::from_str(&operation_id),
                if command.action() == LegacyOrganizationLibraryActionV1::CreateOrganization {
                    JsValue::NULL
                } else {
                    JsValue::from_str(&plan.organization_id)
                },
                JsValue::from_str(&actor_id),
                JsValue::from_str(action),
                JsValue::from_str(&key_digest),
                JsValue::from_str(&command.request_digest_hex()),
                JsValue::from_f64(self.now_ms as f64),
            ],
        )?);
        statements.push(self.statement(
            BROWSER_GRANT_ASSERT_SQL,
            vec![
                JsValue::from_str(&operation_id),
                JsValue::from_str(&fence.mutation_grant_id().to_string()),
                JsValue::from_str(&fence.session_id().to_string()),
                JsValue::from_str(&actor_id),
                JsValue::from_f64(self.now_ms as f64),
            ],
        )?);
        statements.push(self.authority_assertion(&operation_id, &actor_id, &plan.authority)?);
        for mutation in plan.mutations.drain(..) {
            statements.push(self.statement(mutation.sql, mutation.bindings)?);
            if let Some((label, expected)) = mutation.expected_changes {
                statements.push(self.statement(
                    CHANGES_ASSERT_SQL,
                    vec![
                        JsValue::from_str(&operation_id),
                        JsValue::from_str(&label),
                        JsValue::from_f64(expected as f64),
                    ],
                )?);
            }
        }

        let has_pending_deletes = !plan.deletes.is_empty();
        let mut effect_order = 0_i64;
        for put in &plan.puts {
            statements.push(self.r2_effect_insert(R2EffectInsert {
                operation_id: &operation_id,
                order: effect_order,
                kind: "put",
                object_key: &put.object_key,
                checksum: Some(&put.image.checksum_sha256),
                content_type: Some(&put.image.content_type),
                state: "applied",
                applied_at_ms: Some(self.now_ms),
            })?);
            effect_order += 1;
        }
        for delete in &plan.deletes {
            statements.push(self.r2_effect_insert(R2EffectInsert {
                operation_id: &operation_id,
                order: effect_order,
                kind: delete.kind,
                object_key: &delete.object_key,
                checksum: None,
                content_type: None,
                state: "pending",
                applied_at_ms: None,
            })?);
            effect_order += 1;
        }
        if has_pending_deletes {
            statements.push(self.statement(
                OPERATION_SET_PENDING_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&plan.organization_id),
                    JsValue::from_str(&result_json),
                    JsValue::from_str(&effects_json),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?);
        } else {
            statements.push(self.statement(
                OPERATION_COMPLETE_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&plan.organization_id),
                    JsValue::from_str(&result_json),
                    JsValue::from_str(&effects_json),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?);
        }
        statements.push(self.statement(
            AUDIT_INSERT_SQL,
            vec![
                JsValue::from_str(&Uuid::new_v4().to_string()),
                JsValue::from_str(&operation_id),
                JsValue::from_str(&plan.organization_id),
                JsValue::from_str(&digest_text(&actor_id)),
                JsValue::from_str(action),
                JsValue::from_str(&command.request_digest_hex()),
                JsValue::from_f64(self.now_ms as f64),
            ],
        )?);
        statements.push(self.statement(
            BROWSER_GRANT_DELETE_SQL,
            vec![
                JsValue::from_str(&fence.mutation_grant_id().to_string()),
                JsValue::from_str(&fence.session_id().to_string()),
                JsValue::from_str(&actor_id),
            ],
        )?);
        statements.push(self.statement(
            CHANGES_ASSERT_SQL,
            vec![
                JsValue::from_str(&operation_id),
                JsValue::from_str("browser_grant_consumed"),
                JsValue::from_f64(1.0),
            ],
        )?);
        statements.push(self.statement(
            ASSERTION_CLEANUP_SQL,
            vec![JsValue::from_str(&operation_id)],
        )?);
        self.batch(statements).await?;

        if has_pending_deletes {
            self.resume_r2_effects(&operation_id).await?;
            self.finish_pending(
                &operation_id,
                &plan.organization_id,
                &result_json,
                &effects_json,
            )
            .await?;
        }
        Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Applied(receipt))
    }
}

impl D1R2LegacyOrganizationLibraryPortV1<'_> {
    async fn prepare_plan(
        &self,
        command: &LegacyOrganizationLibraryCommandV1,
        operation_id: &str,
    ) -> AtomicResult<ExecutionPlan> {
        use LegacyOrganizationLibraryInputV1 as Input;

        let actor_id = command
            .actor_id()
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?
            .to_string();
        let active_organization_id = command.active_organization_id().map(|id| id.to_string());

        match command.input() {
            Input::SetCollectionLogo {
                legacy_collection_id,
                kind,
                remove,
                image,
            } => {
                let organization_id = active_organization_id
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                let collection_id = map_legacy_id(legacy_collection_id)?;
                let authority = match kind {
                    LegacyCollectionKindV1::Space => {
                        let row = self
                            .space_row(&actor_id, &organization_id, &collection_id)
                            .await?;
                        require_user_pro(
                            row.owner_subscription_status.as_deref(),
                            row.owner_third_party_subscription_id.as_deref(),
                        )?;
                        AuthorityFence::Space(row)
                    }
                    LegacyCollectionKindV1::Folder => {
                        let row = self
                            .folder_row(&actor_id, &organization_id, &collection_id)
                            .await?;
                        require_user_pro(
                            row.owner_subscription_status.as_deref(),
                            row.owner_third_party_subscription_id.as_deref(),
                        )?;
                        AuthorityFence::Folder(row)
                    }
                };
                let existing_settings = match &authority {
                    AuthorityFence::Space(row) => &row.settings_json,
                    AuthorityFence::Folder(row) => &row.settings_json,
                    _ => return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt),
                };
                let old_key = json_string_path(existing_settings, &["publicPage", "logoUrl"])?
                    .filter(|key| valid_r2_key(key));
                let new_key = image.as_ref().map(|payload| {
                    deterministic_image_key(
                        &format!(
                            "organizations/{organization_id}/collections/{collection_id}/logo"
                        ),
                        payload,
                    )
                });
                if *remove != new_key.is_none() {
                    return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
                }
                let puts = image
                    .as_ref()
                    .zip(new_key.as_ref())
                    .map(|(image, object_key)| PutEffect {
                        object_key: object_key.clone(),
                        image: image.clone(),
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                let deletes = old_key
                    .filter(|old| Some(old) != new_key.as_ref())
                    .map(|object_key| DeleteEffect {
                        kind: "delete",
                        object_key,
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                let sql = match kind {
                    LegacyCollectionKindV1::Space => SET_SPACE_LOGO_SQL,
                    LegacyCollectionKindV1::Folder => SET_FOLDER_LOGO_SQL,
                };
                Ok(ExecutionPlan {
                    organization_id,
                    authority,
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: storage_effects(
                        vec![
                            "/dashboard".into(),
                            format!("/dashboard/spaces/{legacy_collection_id}"),
                            format!("/dashboard/folder/{legacy_collection_id}"),
                            format!("/c/{legacy_collection_id}"),
                        ],
                        &puts,
                        &deletes,
                    ),
                    mutations: vec![MutationStatement {
                        sql,
                        bindings: vec![
                            JsValue::from_str(&collection_id),
                            option_string(new_key.as_deref()),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("collection_logo_updated".into(), 1)),
                    }],
                    puts,
                    deletes,
                })
            }
            Input::VerifyCollectionPassword { .. } => {
                Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)
            }
            Input::SetSpaceCollectionVisibility {
                legacy_space_id,
                public,
                settings_patch,
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let space_id = map_legacy_id(legacy_space_id)?;
                let row = self
                    .space_row(&actor_id, &organization_id, &space_id)
                    .await?;
                if (public == &Some(true) && row.is_public == 0) || settings_patch.is_some() {
                    require_user_pro(
                        row.owner_subscription_status.as_deref(),
                        row.owner_third_party_subscription_id.as_deref(),
                    )?;
                }
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Space(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(vec![
                        "/dashboard".into(),
                        format!("/dashboard/spaces/{legacy_space_id}"),
                        format!("/c/{legacy_space_id}"),
                    ]),
                    mutations: vec![MutationStatement {
                        sql: SET_SPACE_VISIBILITY_SQL,
                        bindings: vec![
                            JsValue::from_str(&space_id),
                            public.map_or(JsValue::NULL, |value| {
                                JsValue::from_f64(if value { 1.0 } else { 0.0 })
                            }),
                            settings_patch
                                .as_ref()
                                .map(serde_json::to_string)
                                .transpose()
                                .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?
                                .as_deref()
                                .map_or(JsValue::NULL, JsValue::from_str),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("space_visibility_updated".into(), 1)),
                    }],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::DeleteSpace { legacy_space_id } => {
                let organization_id = require_active_id(active_organization_id)?;
                let space_id = map_legacy_id(legacy_space_id)?;
                let row = self
                    .space_row(&actor_id, &organization_id, &space_id)
                    .await?;
                let deletes = vec![DeleteEffect {
                    kind: "delete_prefix",
                    object_key: format!("organizations/{organization_id}/spaces/{space_id}/"),
                }];
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Space(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: storage_effects(
                        vec!["/dashboard".into(), "/dashboard/spaces".into()],
                        &[],
                        &deletes,
                    ),
                    mutations: vec![MutationStatement {
                        sql: DELETE_SPACE_SQL,
                        bindings: vec![JsValue::from_str(&space_id)],
                        expected_changes: Some(("space_deleted".into(), 1)),
                    }],
                    puts: vec![],
                    deletes,
                })
            }
            Input::GetOrganizationSsoData { .. } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self
                    .active_organization_row(&actor_id, &organization_id)
                    .await?;
                let workos_organization_id = row
                    .legacy_workos_organization_id
                    .clone()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::NotFound)?;
                let connection_id = row
                    .legacy_workos_connection_id
                    .clone()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::NotFound)?;
                let name = row.name.clone();
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Organization {
                        row,
                        mode: "member",
                    },
                    result: LegacyOrganizationLibraryResultV1::OrganizationSsoData {
                        organization_id: workos_organization_id,
                        connection_id,
                        name,
                    },
                    effects: basic_effects(vec![]),
                    mutations: vec![],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::RemoveOrganizationMember {
                legacy_member_id, ..
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self
                    .member_row(&actor_id, &organization_id, legacy_member_id)
                    .await?;
                require_can_mutate_member(&row, &actor_id, true)?;
                let email_digest = digest_text(&row.target_email.to_ascii_lowercase());
                let target_user_id = row.target_user_id.clone();
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::Member(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(vec![
                        "/dashboard/settings/organization".into(),
                        "/dashboard".into(),
                    ]),
                    mutations: vec![
                        MutationStatement {
                            sql: REMOVE_MEMBER_SPACE_MEMBERSHIPS_SQL,
                            bindings: vec![
                                JsValue::from_str(&target_user_id),
                                JsValue::from_str(&organization_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: None,
                        },
                        MutationStatement {
                            sql: REMOVE_MEMBER_INVITES_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&email_digest),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: None,
                        },
                        MutationStatement {
                            sql: REMOVE_MEMBER_ALIAS_SQL,
                            bindings: vec![
                                JsValue::from_str(legacy_member_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("member_alias_removed".into(), 1)),
                        },
                        MutationStatement {
                            sql: REMOVE_MEMBER_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&target_user_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("member_removed".into(), 1)),
                        },
                    ],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::UpdateOrganizationSettings { settings } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                let normalized_settings = normalize_organization_settings(settings)?;
                let next_settings = if user_is_pro(
                    row.legacy_stripe_subscription_status.as_deref(),
                    row.legacy_third_party_stripe_subscription_id.as_deref(),
                ) {
                    normalized_settings
                } else {
                    preserve_pro_settings(&normalized_settings, &row.settings_json)?
                };
                let settings_json = serde_json::to_string(&next_settings)
                    .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                Ok(organization_mutation_plan(
                    organization_id,
                    row,
                    LegacyOrganizationLibraryResultV1::Success,
                    vec![
                        "/dashboard/caps".into(),
                        "/dashboard/settings/organization".into(),
                        "/dashboard/settings/organization/preferences".into(),
                    ],
                    MutationStatement {
                        sql: UPDATE_ORGANIZATION_SETTINGS_SQL,
                        bindings: vec![
                            JsValue::from_str(&require_active_id(
                                command.active_organization_id().map(|id| id.to_string()),
                            )?),
                            JsValue::from_str(&settings_json),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("organization_settings_updated".into(), 1)),
                    },
                ))
            }
            Input::HideShareableLinkCapLogo { .. } => {
                self.branding_patch_plan(
                    &actor_id,
                    require_active_id(active_organization_id)?,
                    operation_id,
                    Some(json!({"hideShareableLinkCapLogo": true})),
                    None,
                    None,
                )
                .await
            }
            Input::RemoveShareableLinkIcon { .. } => {
                self.branding_patch_plan(
                    &actor_id,
                    require_active_id(active_organization_id)?,
                    operation_id,
                    None,
                    Some(true),
                    None,
                )
                .await
            }
            Input::SelectShareableLinkBrandingOrganization {
                legacy_organization_id,
            } => {
                let organization_id = map_legacy_id(legacy_organization_id)?;
                let row = self.target_manager_row(&actor_id, &organization_id).await?;
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::TargetManager(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(vec!["/dashboard".into()]),
                    mutations: vec![MutationStatement {
                        sql: SELECT_ACTIVE_ORGANIZATION_SQL,
                        bindings: vec![
                            JsValue::from_str(&actor_id),
                            JsValue::from_str(&organization_id),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("active_organization_selected".into(), 1)),
                    }],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::UpdateShareableLinkIconPreference {
                use_organization_icon,
                ..
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                require_user_pro(
                    row.legacy_stripe_subscription_status.as_deref(),
                    row.legacy_third_party_stripe_subscription_id.as_deref(),
                )?;
                if *use_organization_icon && row.legacy_icon_key.is_none() {
                    return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
                }
                self.branding_patch_plan_with_row(
                    organization_id,
                    row,
                    operation_id,
                    Some(json!({"shareableLinkUseOrganizationIcon": use_organization_icon})),
                    None,
                    None,
                )
            }
            Input::UploadShareableLinkIcon { image, .. } => {
                let organization_id = require_active_id(active_organization_id)?;
                self.branding_patch_plan(
                    &actor_id,
                    organization_id,
                    operation_id,
                    None,
                    Some(true),
                    Some(image.clone()),
                )
                .await
            }
            Input::ConnectOrganizationGoogleDrive { .. } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                require_user_pro(
                    row.legacy_stripe_subscription_status.as_deref(),
                    row.legacy_third_party_stripe_subscription_id.as_deref(),
                )?;
                let url = self.google_drive_authorization_url(&actor_id, &organization_id)?;
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Organization {
                        row,
                        mode: "manager",
                    },
                    result: LegacyOrganizationLibraryResultV1::GoogleDriveAuthorization { url },
                    effects: basic_effects(vec![]),
                    mutations: vec![],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::DisconnectOrganizationGoogleDrive { .. } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                require_user_pro(
                    row.legacy_stripe_subscription_status.as_deref(),
                    row.legacy_third_party_stripe_subscription_id.as_deref(),
                )?;
                Ok(organization_mutation_plan(
                    organization_id.clone(),
                    row,
                    LegacyOrganizationLibraryResultV1::Success,
                    storage_paths(),
                    MutationStatement {
                        sql: DISCONNECT_GOOGLE_DRIVE_SQL,
                        bindings: vec![
                            JsValue::from_str(&organization_id),
                            JsValue::from_f64(self.now_ms as f64),
                            JsValue::from_str(operation_id),
                        ],
                        expected_changes: None,
                    },
                ))
            }
            Input::GetOrganizationStorageSettings {
                legacy_organization_id,
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                let storage = self.storage_rows(&organization_id).await?;
                let settings = storage_settings_json(
                    legacy_organization_id,
                    &row.name,
                    &storage,
                    self.config,
                )?;
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Organization {
                        row,
                        mode: "manager",
                    },
                    result: LegacyOrganizationLibraryResultV1::OrganizationStorageSettings {
                        settings,
                    },
                    effects: basic_effects(vec![]),
                    mutations: vec![],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::SetOrganizationStorageProvider { provider, .. } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                require_user_pro(
                    row.legacy_stripe_subscription_status.as_deref(),
                    row.legacy_third_party_stripe_subscription_id.as_deref(),
                )?;
                let storage = self.storage_rows(&organization_id).await?;
                let providers = match provider {
                    LegacyOrganizationStorageProviderV1::S3 => {
                        if !storage.iter().any(|item| {
                            matches!(item.provider.as_str(), "r2" | "s3_compatible" | "minio")
                                && item.state != "revoked"
                        }) {
                            return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
                        }
                        json!(["r2", "s3_compatible", "minio"])
                    }
                    LegacyOrganizationStorageProviderV1::GoogleDrive => {
                        let configured = storage.iter().any(|item| {
                            item.provider == "google_drive"
                                && item.state != "revoked"
                                && google_drive_location_configured(&item.capabilities_json)
                        });
                        if !configured {
                            return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
                        }
                        json!(["google_drive"])
                    }
                };
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::Organization {
                        row,
                        mode: "manager",
                    },
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(storage_paths()),
                    mutations: vec![
                        MutationStatement {
                            sql: DISABLE_STORAGE_INTEGRATIONS_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: None,
                        },
                        MutationStatement {
                            sql: ENABLE_STORAGE_INTEGRATION_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&providers.to_string()),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("storage_provider_enabled".into(), 1)),
                        },
                    ],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::ToggleProSeat {
                legacy_member_id,
                enable,
                ..
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self
                    .member_row(&actor_id, &organization_id, legacy_member_id)
                    .await?;
                require_can_mutate_member(&row, &actor_id, false)?;
                if row.has_pro_seat == i64::from(*enable) {
                    return Ok(ExecutionPlan {
                        organization_id,
                        authority: AuthorityFence::Member(row),
                        result: LegacyOrganizationLibraryResultV1::Success,
                        effects: basic_effects(vec!["/dashboard/settings/organization".into()]),
                        mutations: vec![],
                        puts: vec![],
                        deletes: vec![],
                    });
                }
                if *enable {
                    let count = self.pro_seat_count(&organization_id).await?;
                    let quota = seat_provider(&row).0.max(1);
                    if count >= quota {
                        return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
                    }
                }
                let target_user_id = row.target_user_id.clone();
                let subscription_link = if *enable {
                    seat_provider(&row).1
                } else {
                    self.remaining_pro_subscription(&target_user_id, &organization_id)
                        .await?
                };
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::Member(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(vec!["/dashboard/settings/organization".into()]),
                    mutations: vec![
                        MutationStatement {
                            sql: TOGGLE_PRO_SEAT_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&target_user_id),
                                JsValue::from_f64(if *enable { 1.0 } else { 0.0 }),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("pro_seat_toggled".into(), 1)),
                        },
                        MutationStatement {
                            sql: UPDATE_MEMBER_SUBSCRIPTION_LINK_SQL,
                            bindings: vec![
                                JsValue::from_str(&target_user_id),
                                option_string(subscription_link.as_deref()),
                                JsValue::from_f64(self.now_ms as f64),
                            ],
                            expected_changes: Some(("member_subscription_link_updated".into(), 1)),
                        },
                    ],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::UpdateOrganizationDetails {
                organization_name,
                allowed_email_domain,
                ..
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self.active_manager_row(&actor_id, &organization_id).await?;
                Ok(organization_mutation_plan(
                    organization_id.clone(),
                    row,
                    LegacyOrganizationLibraryResultV1::Success,
                    vec!["/dashboard/settings/organization".into()],
                    MutationStatement {
                        sql: UPDATE_ORGANIZATION_DETAILS_SQL,
                        bindings: vec![
                            JsValue::from_str(&organization_id),
                            option_string(organization_name.as_deref()),
                            JsValue::from_f64(if allowed_email_domain.is_some() {
                                1.0
                            } else {
                                0.0
                            }),
                            option_string(allowed_email_domain.as_deref()),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("organization_details_updated".into(), 1)),
                    },
                ))
            }
            Input::UpdateOrganizationMemberRole {
                legacy_member_id,
                role,
                ..
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let row = self
                    .member_row(&actor_id, &organization_id, legacy_member_id)
                    .await?;
                let role = role.to_ascii_lowercase();
                require_can_change_role(&row, &actor_id, &role)?;
                let target_user_id = row.target_user_id.clone();
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::Member(row),
                    result: LegacyOrganizationLibraryResultV1::Success,
                    effects: basic_effects(vec![
                        "/dashboard/settings/organization".into(),
                        "/dashboard".into(),
                    ]),
                    mutations: vec![MutationStatement {
                        sql: UPDATE_MEMBER_ROLE_SQL,
                        bindings: vec![
                            JsValue::from_str(&organization_id),
                            JsValue::from_str(&target_user_id),
                            JsValue::from_str(&role),
                            JsValue::from_f64(self.now_ms as f64),
                            JsValue::from_str(operation_id),
                        ],
                        expected_changes: Some(("member_role_updated".into(), 1)),
                    }],
                    puts: vec![],
                    deletes: vec![],
                })
            }
            Input::UploadSpaceIcon {
                legacy_space_id,
                image,
            } => {
                let organization_id = require_active_id(active_organization_id)?;
                let space_id = map_legacy_id(legacy_space_id)?;
                let row = self
                    .space_row(&actor_id, &organization_id, &space_id)
                    .await?;
                let object_key = deterministic_image_key(
                    &format!("organizations/{organization_id}/spaces/{space_id}/icon"),
                    image,
                );
                let puts = vec![PutEffect {
                    object_key: object_key.clone(),
                    image: image.clone(),
                }];
                let deletes = row
                    .legacy_icon_key
                    .clone()
                    .filter(|old| valid_r2_key(old) && old != &object_key)
                    .map(|object_key| DeleteEffect {
                        kind: "delete",
                        object_key,
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                Ok(ExecutionPlan {
                    organization_id,
                    authority: AuthorityFence::Space(row),
                    result: LegacyOrganizationLibraryResultV1::IconUploaded {
                        object_key: object_key.clone(),
                    },
                    effects: storage_effects(vec!["/dashboard".into()], &puts, &deletes),
                    mutations: vec![MutationStatement {
                        sql: UPLOAD_SPACE_ICON_SQL,
                        bindings: vec![
                            JsValue::from_str(&space_id),
                            JsValue::from_str(&object_key),
                            JsValue::from_str(operation_id),
                            JsValue::from_f64(self.now_ms as f64),
                        ],
                        expected_changes: Some(("space_icon_updated".into(), 1)),
                    }],
                    puts,
                    deletes,
                })
            }
            Input::CreateOrganization { name, icon } => {
                let legacy_organization_id = command
                    .deterministic_legacy_organization_id()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                let organization_id = map_legacy_id(&legacy_organization_id)?;
                if self.organization_name_exists(name).await? {
                    return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
                }
                let actor = self.actor_row(&actor_id).await?;
                let object_key = icon.as_ref().map(|image| {
                    deterministic_image_key(&format!("organizations/{organization_id}/icon"), image)
                });
                let puts = icon
                    .as_ref()
                    .zip(object_key.as_ref())
                    .map(|(image, object_key)| PutEffect {
                        object_key: object_key.clone(),
                        image: image.clone(),
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                Ok(ExecutionPlan {
                    organization_id: organization_id.clone(),
                    authority: AuthorityFence::Actor(actor),
                    result: LegacyOrganizationLibraryResultV1::OrganizationCreated {
                        legacy_organization_id: legacy_organization_id.clone(),
                    },
                    effects: storage_effects(vec!["/dashboard".into()], &puts, &[]),
                    mutations: vec![
                        MutationStatement {
                            sql: ORGANIZATION_NAME_ASSERT_SQL,
                            bindings: vec![
                                JsValue::from_str(operation_id),
                                JsValue::from_str(name),
                            ],
                            expected_changes: None,
                        },
                        MutationStatement {
                            sql: CREATE_ORGANIZATION_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&actor_id),
                                JsValue::from_str(name),
                                option_string(object_key.as_deref()),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("organization_created".into(), 1)),
                        },
                        MutationStatement {
                            sql: CREATE_ORGANIZATION_MEMBER_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&actor_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("organization_owner_created".into(), 1)),
                        },
                        MutationStatement {
                            sql: CREATE_ORGANIZATION_ALIAS_SQL,
                            bindings: vec![
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(&legacy_organization_id),
                                JsValue::from_f64(self.now_ms as f64),
                                JsValue::from_str(operation_id),
                            ],
                            expected_changes: Some(("organization_alias_created".into(), 1)),
                        },
                        MutationStatement {
                            sql: SELECT_ACTIVE_ORGANIZATION_SQL,
                            bindings: vec![
                                JsValue::from_str(&actor_id),
                                JsValue::from_str(&organization_id),
                                JsValue::from_str(operation_id),
                                JsValue::from_f64(self.now_ms as f64),
                            ],
                            expected_changes: Some(("created_organization_selected".into(), 1)),
                        },
                    ],
                    puts,
                    deletes: vec![],
                })
            }
        }
    }

    async fn active_organization_row(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<ActiveOrganizationRow> {
        one_row(
            self.rows(
                ACTIVE_ORGANIZATION_SNAPSHOT_SQL,
                vec![
                    JsValue::from_str(actor_id),
                    JsValue::from_str(organization_id),
                ],
            )
            .await?,
        )
    }

    async fn active_manager_row(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<ActiveOrganizationRow> {
        let row = self
            .active_organization_row(actor_id, organization_id)
            .await?;
        if row.owner_id != actor_id
            && !(row.actor_membership_state.as_deref() == Some("active")
                && row.actor_role.as_deref() == Some("admin"))
        {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
        }
        Ok(row)
    }

    async fn target_manager_row(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<TargetManagerRow> {
        one_row(
            self.rows(
                TARGET_ORGANIZATION_MANAGER_SNAPSHOT_SQL,
                vec![
                    JsValue::from_str(actor_id),
                    JsValue::from_str(organization_id),
                ],
            )
            .await?,
        )
    }

    async fn actor_row(&self, actor_id: &str) -> AtomicResult<ActorRow> {
        one_row(
            self.rows(ACTOR_SNAPSHOT_SQL, vec![JsValue::from_str(actor_id)])
                .await?,
        )
    }

    async fn space_row(
        &self,
        actor_id: &str,
        organization_id: &str,
        space_id: &str,
    ) -> AtomicResult<SpaceRow> {
        one_row(
            self.rows(
                SPACE_SNAPSHOT_SQL,
                vec![
                    JsValue::from_str(actor_id),
                    JsValue::from_str(organization_id),
                    JsValue::from_str(space_id),
                ],
            )
            .await?,
        )
    }

    async fn folder_row(
        &self,
        actor_id: &str,
        organization_id: &str,
        folder_id: &str,
    ) -> AtomicResult<FolderRow> {
        one_row(
            self.rows(
                FOLDER_SNAPSHOT_SQL,
                vec![
                    JsValue::from_str(actor_id),
                    JsValue::from_str(organization_id),
                    JsValue::from_str(folder_id),
                ],
            )
            .await?,
        )
    }

    async fn member_row(
        &self,
        actor_id: &str,
        organization_id: &str,
        legacy_member_id: &str,
    ) -> AtomicResult<MemberRow> {
        one_row(
            self.rows(
                MEMBER_SNAPSHOT_SQL,
                vec![
                    JsValue::from_str(actor_id),
                    JsValue::from_str(organization_id),
                    JsValue::from_str(legacy_member_id),
                ],
            )
            .await?,
        )
    }

    async fn storage_rows(&self, organization_id: &str) -> AtomicResult<Vec<StorageRow>> {
        self.rows(
            STORAGE_SNAPSHOT_SQL,
            vec![JsValue::from_str(organization_id)],
        )
        .await
    }

    async fn pro_seat_count(&self, organization_id: &str) -> AtomicResult<i64> {
        let row: CountRow = one_row(
            self.rows(PRO_SEAT_COUNT_SQL, vec![JsValue::from_str(organization_id)])
                .await?,
        )?;
        Ok(row.assigned_count)
    }

    async fn remaining_pro_subscription(
        &self,
        user_id: &str,
        excluded_organization_id: &str,
    ) -> AtomicResult<Option<String>> {
        #[derive(Deserialize)]
        struct SubscriptionRow {
            subscription_id: Option<String>,
        }
        let rows = self
            .rows::<SubscriptionRow>(
                REMAINING_PRO_SUBSCRIPTION_SQL,
                vec![
                    JsValue::from_str(user_id),
                    JsValue::from_str(excluded_organization_id),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        Ok(rows.into_iter().next().and_then(|row| row.subscription_id))
    }

    async fn organization_name_exists(&self, name: &str) -> AtomicResult<bool> {
        #[derive(Deserialize)]
        struct NameCount {
            existing_count: i64,
        }
        let row: NameCount = one_row(
            self.rows(
                "SELECT COUNT(*) AS existing_count FROM organizations WHERE status <> 'deleted' AND COALESCE(legacy_user_account_name, name) = ?1",
                vec![JsValue::from_str(name)],
            )
            .await?,
        )?;
        Ok(row.existing_count != 0)
    }

    async fn branding_patch_plan(
        &self,
        actor_id: &str,
        organization_id: String,
        operation_id: &str,
        settings_patch: Option<Value>,
        write_icon: Option<bool>,
        image: Option<LegacyImagePayloadV1>,
    ) -> AtomicResult<ExecutionPlan> {
        let row = self.active_manager_row(actor_id, &organization_id).await?;
        require_user_pro(
            row.legacy_stripe_subscription_status.as_deref(),
            row.legacy_third_party_stripe_subscription_id.as_deref(),
        )?;
        self.branding_patch_plan_with_row(
            organization_id,
            row,
            operation_id,
            settings_patch,
            write_icon,
            image,
        )
    }

    fn branding_patch_plan_with_row(
        &self,
        organization_id: String,
        row: ActiveOrganizationRow,
        operation_id: &str,
        settings_patch: Option<Value>,
        write_icon: Option<bool>,
        image: Option<LegacyImagePayloadV1>,
    ) -> AtomicResult<ExecutionPlan> {
        let object_key = image.as_ref().map(|payload| {
            deterministic_image_key(
                &format!("organizations/{organization_id}/shareable-links/icon"),
                payload,
            )
        });
        let puts = image
            .as_ref()
            .zip(object_key.as_ref())
            .map(|(image, object_key)| PutEffect {
                object_key: object_key.clone(),
                image: image.clone(),
            })
            .into_iter()
            .collect::<Vec<_>>();
        let deletes = write_icon
            .is_some()
            .then(|| row.legacy_shareable_link_icon_key.clone())
            .flatten()
            .filter(|old| valid_r2_key(old) && Some(old) != object_key.as_ref())
            .map(|object_key| DeleteEffect {
                kind: "delete",
                object_key,
            })
            .into_iter()
            .collect::<Vec<_>>();
        let patch_json = settings_patch
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
        Ok(ExecutionPlan {
            organization_id: organization_id.clone(),
            authority: AuthorityFence::Organization {
                row,
                mode: "manager",
            },
            result: LegacyOrganizationLibraryResultV1::Success,
            effects: storage_effects(branding_paths(), &puts, &deletes),
            mutations: vec![MutationStatement {
                sql: PATCH_ORGANIZATION_BRANDING_SQL,
                bindings: vec![
                    JsValue::from_str(&organization_id),
                    option_string(patch_json.as_deref()),
                    JsValue::from_f64(if write_icon.is_some() { 1.0 } else { 0.0 }),
                    option_string(object_key.as_deref()),
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(self.now_ms as f64),
                ],
                expected_changes: Some(("organization_branding_updated".into(), 1)),
            }],
            puts,
            deletes,
        })
    }

    fn google_drive_authorization_url(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<String> {
        if self.config.google_client_id.is_empty()
            || self.config.google_redirect_uri.is_empty()
            || self.config.google_auth_base_url.is_empty()
            || self.config.state_secret.len() < 32
        {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Unavailable);
        }
        let state_payload = serde_json::to_vec(&json!({
            "userId": actor_id,
            "expiresAt": self.now_ms.saturating_add(10 * 60 * 1000),
            "scope": "organization",
            "organizationId": organization_id,
        }))
        .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
        let payload = URL_SAFE_NO_PAD.encode(state_payload);
        let mut signer = Hmac::<Sha256>::new_from_slice(&self.config.state_secret)
            .expect("HMAC accepts any key length");
        signer.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes());
        let state = format!("{payload}.{signature}");
        Ok(format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&access_type=offline&prompt=consent&scope={}&state={}&include_granted_scopes=true",
            self.config.google_auth_base_url,
            percent_encode(&self.config.google_client_id),
            percent_encode(&self.config.google_redirect_uri),
            percent_encode("https://www.googleapis.com/auth/drive.file"),
            percent_encode(&state),
        ))
    }

    fn authority_assertion(
        &self,
        operation_id: &str,
        actor_id: &str,
        authority: &AuthorityFence,
    ) -> AtomicResult<D1PreparedStatement> {
        match authority {
            AuthorityFence::Organization { row, mode } => self.statement(
                ORGANIZATION_AUTHORITY_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&row.organization_id),
                    JsValue::from_f64(row.organization_preference_revision as f64),
                    JsValue::from_f64(row.organization_revision as f64),
                    JsValue::from_f64(row.organization_authority_version as f64),
                    JsValue::from_f64(row.legacy_organization_library_revision as f64),
                    JsValue::from_str(mode),
                ],
            ),
            AuthorityFence::TargetManager(row) => self.statement(
                TARGET_MANAGER_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&row.organization_id),
                    JsValue::from_f64(row.organization_preference_revision as f64),
                    JsValue::from_f64(row.organization_revision as f64),
                    JsValue::from_f64(row.organization_authority_version as f64),
                ],
            ),
            AuthorityFence::Actor(row) => self.statement(
                ACTOR_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&row.id),
                    JsValue::from_f64(row.organization_preference_revision as f64),
                ],
            ),
            AuthorityFence::Space(row) => self.statement(
                SPACE_AUTHORITY_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&row.organization_id),
                    JsValue::from_str(&row.space_id),
                    JsValue::from_f64(row.organization_revision as f64),
                    JsValue::from_f64(row.organization_authority_version as f64),
                    JsValue::from_f64(row.space_revision as f64),
                    JsValue::from_f64(row.space_authority_version as f64),
                    JsValue::from_f64(row.legacy_organization_library_revision as f64),
                ],
            ),
            AuthorityFence::Folder(row) => self.statement(
                FOLDER_AUTHORITY_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&row.organization_id),
                    JsValue::from_str(&row.folder_id),
                    JsValue::from_f64(row.organization_revision as f64),
                    JsValue::from_f64(row.organization_authority_version as f64),
                    JsValue::from_f64(row.folder_revision as f64),
                    JsValue::from_f64(row.tree_revision as f64),
                    JsValue::from_f64(row.legacy_organization_library_revision as f64),
                ],
            ),
            AuthorityFence::Member(row) => self.statement(
                MEMBER_AUTHORITY_ASSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&row.organization_id),
                    JsValue::from_str(&row.legacy_member_id),
                    JsValue::from_f64(row.target_revision as f64),
                    JsValue::from_f64(row.target_authority_version as f64),
                ],
            ),
        }
    }

    async fn verify_collection_password(
        &self,
        command: &LegacyOrganizationLibraryCommandV1,
    ) -> AtomicResult<LegacyOrganizationLibraryAtomicOutcomeV1> {
        let LegacyOrganizationLibraryInputV1::VerifyCollectionPassword {
            legacy_collection_id,
            password,
        } = command.input()
        else {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        };
        let collection_id = map_legacy_id(legacy_collection_id)?;
        let mut rows = self
            .rows::<PasswordRow>(
                PASSWORD_SNAPSHOT_SQL,
                vec![JsValue::from_str(&collection_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        let verified_hash = rows.pop().filter(valid_password_row).and_then(|row| {
            row.password_hash
                .filter(|stored| verify_password(stored, password))
        });
        let verified = verified_hash.is_some();
        let result = verified_hash.map_or(
            LegacyOrganizationLibraryResultV1::PasswordRejected,
            |password_hash| LegacyOrganizationLibraryResultV1::PasswordVerified { password_hash },
        );
        let receipt = LegacyOrganizationLibraryReceiptV1 {
            action: command.action(),
            request_digest: *command.request_digest(),
            result,
            effects: LegacyOrganizationLibraryEffectsV1 {
                invalidation_paths: if verified {
                    vec![format!("/c/{legacy_collection_id}")]
                } else {
                    vec![]
                },
                set_verified_password_cookie: verified,
                r2_keys_written: vec![],
                r2_keys_deleted: vec![],
            },
        };
        if !receipt.matches(command) {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Applied(receipt))
    }

    async fn resume(
        &self,
        command: &LegacyOrganizationLibraryCommandV1,
        operation: &OperationRow,
    ) -> AtomicResult<LegacyOrganizationLibraryAtomicOutcomeV1> {
        if operation.actor_id
            != command
                .actor_id()
                .map(|id| id.to_string())
                .unwrap_or_default()
            || operation.action != action_name(command.action())
        {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        if operation.request_digest != command.request_digest_hex() {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict);
        }
        match operation.state.as_str() {
            "claimed" => Err(LegacyOrganizationLibraryAtomicErrorV1::InFlight),
            "storage_pending" => {
                self.resume_r2_effects(&operation.operation_id).await?;
                let result_json = operation
                    .result_json
                    .as_deref()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                let effects_json = operation
                    .effects_json
                    .as_deref()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                let organization_id = operation
                    .organization_id
                    .as_deref()
                    .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
                self.finish_pending(
                    &operation.operation_id,
                    organization_id,
                    result_json,
                    effects_json,
                )
                .await?;
                Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Replay(
                    receipt_from_row(command, operation)?,
                ))
            }
            "complete" => Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Replay(
                receipt_from_row(command, operation)?,
            )),
            _ => Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt),
        }
    }

    async fn finish_pending(
        &self,
        operation_id: &str,
        organization_id: &str,
        result_json: &str,
        effects_json: &str,
    ) -> AtomicResult<()> {
        self.batch(vec![self.statement(
            OPERATION_COMPLETE_SQL,
            vec![
                JsValue::from_str(operation_id),
                JsValue::from_str(organization_id),
                JsValue::from_str(result_json),
                JsValue::from_str(effects_json),
                JsValue::from_f64(self.now_ms as f64),
            ],
        )?])
        .await
    }

    fn r2_effect_insert(&self, effect: R2EffectInsert<'_>) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            R2_EFFECT_INSERT_SQL,
            vec![
                JsValue::from_str(effect.operation_id),
                JsValue::from_f64(effect.order as f64),
                JsValue::from_str(effect.kind),
                JsValue::from_str(effect.object_key),
                option_string(effect.checksum),
                option_string(effect.content_type),
                JsValue::from_str(effect.state),
                effect
                    .applied_at_ms
                    .map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64)),
            ],
        )
    }

    async fn apply_put(&self, put: &PutEffect) -> AtomicResult<()> {
        self.bucket
            .put(&put.object_key, put.image.bytes.clone())
            .http_metadata(HttpMetadata {
                content_type: Some(put.image.content_type.clone()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .custom_metadata(HashMap::from([(
                "sha256".into(),
                put.image.checksum_sha256.clone(),
            )]))
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?;
        Ok(())
    }

    async fn resume_r2_effects(&self, operation_id: &str) -> AtomicResult<()> {
        let effects = self
            .rows::<R2EffectRow>(
                R2_EFFECTS_PENDING_SQL,
                vec![JsValue::from_str(operation_id)],
            )
            .await?;
        if effects.len() > MAX_R2_PREFIX_OBJECTS {
            return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
        }
        for effect in effects {
            if effect.checksum_sha256.is_some() || effect.content_type.is_some() {
                return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
            }
            match effect.effect_kind.as_str() {
                "delete" => {
                    self.bucket
                        .delete(&effect.object_key)
                        .into_send()
                        .await
                        .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?;
                }
                "delete_prefix" => self.delete_prefix(&effect.object_key).await?,
                _ => return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt),
            }
            self.batch(vec![self.statement(
                R2_EFFECT_APPLIED_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(effect.effect_order as f64),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?])
            .await?;
        }
        Ok(())
    }

    async fn delete_prefix(&self, prefix: &str) -> AtomicResult<()> {
        let mut cursor = None;
        let mut deleted = 0_usize;
        for _ in 0..MAX_R2_PREFIX_PAGES {
            let mut list = self.bucket.list().prefix(prefix).limit(1_000);
            if let Some(value) = cursor.as_deref() {
                list = list.cursor(value);
            }
            let page = list
                .execute()
                .into_send()
                .await
                .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?;
            for object in page.objects() {
                deleted += 1;
                if deleted > MAX_R2_PREFIX_OBJECTS {
                    return Err(LegacyOrganizationLibraryAtomicErrorV1::Unavailable);
                }
                self.bucket
                    .delete(&object.key())
                    .into_send()
                    .await
                    .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Unavailable)?;
            }
            if !page.truncated() {
                return Ok(());
            }
            cursor = page.cursor();
            if cursor.is_none() {
                return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
            }
        }
        Err(LegacyOrganizationLibraryAtomicErrorV1::Unavailable)
    }
}

fn action_name(action: LegacyOrganizationLibraryActionV1) -> &'static str {
    match action {
        LegacyOrganizationLibraryActionV1::SetCollectionLogo => "set_collection_logo",
        LegacyOrganizationLibraryActionV1::VerifyCollectionPassword => "verify_collection_password",
        LegacyOrganizationLibraryActionV1::SetSpaceCollectionVisibility => {
            "set_space_collection_visibility"
        }
        LegacyOrganizationLibraryActionV1::DeleteSpace => "delete_space",
        LegacyOrganizationLibraryActionV1::GetOrganizationSsoData => "get_organization_sso_data",
        LegacyOrganizationLibraryActionV1::RemoveOrganizationMember => "remove_organization_member",
        LegacyOrganizationLibraryActionV1::UpdateOrganizationSettings => {
            "update_organization_settings"
        }
        LegacyOrganizationLibraryActionV1::HideShareableLinkCapLogo => {
            "hide_shareable_link_cap_logo"
        }
        LegacyOrganizationLibraryActionV1::RemoveShareableLinkIcon => "remove_shareable_link_icon",
        LegacyOrganizationLibraryActionV1::SelectShareableLinkBrandingOrganization => {
            "select_shareable_link_branding_organization"
        }
        LegacyOrganizationLibraryActionV1::UpdateShareableLinkIconPreference => {
            "update_shareable_link_icon_preference"
        }
        LegacyOrganizationLibraryActionV1::UploadShareableLinkIcon => "upload_shareable_link_icon",
        LegacyOrganizationLibraryActionV1::ConnectOrganizationGoogleDrive => {
            "connect_organization_google_drive"
        }
        LegacyOrganizationLibraryActionV1::DisconnectOrganizationGoogleDrive => {
            "disconnect_organization_google_drive"
        }
        LegacyOrganizationLibraryActionV1::GetOrganizationStorageSettings => {
            "get_organization_storage_settings"
        }
        LegacyOrganizationLibraryActionV1::SetOrganizationStorageProvider => {
            "set_organization_storage_provider"
        }
        LegacyOrganizationLibraryActionV1::ToggleProSeat => "toggle_pro_seat",
        LegacyOrganizationLibraryActionV1::UpdateOrganizationDetails => {
            "update_organization_details"
        }
        LegacyOrganizationLibraryActionV1::UpdateOrganizationMemberRole => {
            "update_organization_member_role"
        }
        LegacyOrganizationLibraryActionV1::UploadSpaceIcon => "upload_space_icon",
        LegacyOrganizationLibraryActionV1::CreateOrganization => "create_organization",
    }
}

fn map_legacy_id(value: &str) -> AtomicResult<String> {
    LegacyCapNanoId::parse(value.to_owned())
        .map(|id| id.mapped_uuid().to_string())
        .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::NotFound)
}

fn require_active_id(value: Option<String>) -> AtomicResult<String> {
    value.ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)
}

fn one_row<T>(mut rows: Vec<T>) -> AtomicResult<T> {
    if rows.len() > 1 {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
    }
    rows.pop()
        .ok_or(LegacyOrganizationLibraryAtomicErrorV1::NotFound)
}

fn option_string(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn valid_safe_integer(value: i64) -> bool {
    (0..=9_007_199_254_740_991).contains(&value)
}

fn digest_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn map_d1_message(message: &str) -> LegacyOrganizationLibraryAtomicErrorV1 {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("unique constraint")
        || normalized.contains("foreign key constraint")
        || normalized.contains("expected_count = actual_count")
        || normalized.contains("operation_immutable")
        || normalized.contains("r2_immutable")
    {
        LegacyOrganizationLibraryAtomicErrorV1::Conflict
    } else if normalized.contains("not authorized") || normalized.contains("access denied") {
        LegacyOrganizationLibraryAtomicErrorV1::AccessDenied
    } else {
        LegacyOrganizationLibraryAtomicErrorV1::Unavailable
    }
}

fn receipt_from_row(
    command: &LegacyOrganizationLibraryCommandV1,
    operation: &OperationRow,
) -> AtomicResult<LegacyOrganizationLibraryReceiptV1> {
    let result = serde_json::from_str(
        operation
            .result_json
            .as_deref()
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    let effects = serde_json::from_str(
        operation
            .effects_json
            .as_deref()
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    let receipt = LegacyOrganizationLibraryReceiptV1 {
        action: command.action(),
        request_digest: *command.request_digest(),
        result,
        effects,
    };
    if !receipt.matches(command) {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt);
    }
    Ok(receipt)
}

fn valid_password_row(row: &PasswordRow) -> bool {
    Uuid::parse_str(&row.collection_id).is_ok()
        && Uuid::parse_str(&row.organization_id).is_ok()
        && valid_safe_integer(row.password_revision)
        && matches!(row.collection_kind.as_str(), "space" | "folder")
}

fn verify_password(stored: &str, password: &str) -> bool {
    let Ok(decoded) = STANDARD.decode(stored) else {
        return false;
    };
    if decoded.len() != 48 {
        return false;
    }
    let mut actual = [0_u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &decoded[..16], 100_000, &mut actual);
    constant_time_equal_bytes(&actual, &decoded[16..])
}

fn constant_time_equal_bytes(actual: &[u8], expected: &[u8]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let key = b"frame.organization-library.pbkdf2-compare.v1";
    let mut expected_mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    expected_mac.update(expected);
    let expected_tag = expected_mac.finalize().into_bytes();
    let mut actual_mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    actual_mac.update(actual);
    actual_mac.verify_slice(&expected_tag).is_ok()
}

fn has_active_direct_subscription(subscription_id: Option<&str>, status: Option<&str>) -> bool {
    subscription_id.is_some_and(|value| !value.is_empty())
        && matches!(status, Some("active" | "trialing" | "complete" | "paid"))
}

fn user_is_pro(status: Option<&str>, third_party_subscription_id: Option<&str>) -> bool {
    third_party_subscription_id.is_some_and(|value| !value.is_empty())
        || matches!(status, Some("active" | "trialing" | "complete" | "paid"))
}

fn require_user_pro(
    status: Option<&str>,
    third_party_subscription_id: Option<&str>,
) -> AtomicResult<()> {
    if user_is_pro(status, third_party_subscription_id) {
        Ok(())
    } else {
        Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied)
    }
}

fn seat_provider(row: &MemberRow) -> (i64, Option<String>) {
    let actor_active = has_active_direct_subscription(
        row.actor_subscription_id.as_deref(),
        row.actor_subscription_status.as_deref(),
    );
    let owner_active = has_active_direct_subscription(
        row.owner_subscription_id.as_deref(),
        row.owner_subscription_status.as_deref(),
    );
    if actor_active && (!owner_active || row.actor_invite_quota >= row.owner_invite_quota) {
        (row.actor_invite_quota, row.actor_subscription_id.clone())
    } else {
        (row.owner_invite_quota, row.owner_subscription_id.clone())
    }
}

fn require_can_mutate_member(row: &MemberRow, actor_id: &str, removing: bool) -> AtomicResult<()> {
    if row.target_state != "active"
        || row.target_user_id == row.owner_id
        || row.target_role == "owner"
        || (removing && row.target_user_id == actor_id)
    {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
    }
    if actor_id == row.owner_id {
        return Ok(());
    }
    if row.actor_state.as_deref() != Some("active") || row.actor_role.as_deref() != Some("admin") {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
    }
    if removing && (row.target_role == "admin" || row.target_user_id == actor_id) {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
    }
    Ok(())
}

fn require_can_change_role(row: &MemberRow, actor_id: &str, next_role: &str) -> AtomicResult<()> {
    if !matches!(next_role, "admin" | "member")
        || row.target_state != "active"
        || row.target_user_id == row.owner_id
        || row.target_role == "owner"
    {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
    }
    if actor_id == row.owner_id {
        return Ok(());
    }
    if row.actor_state.as_deref() != Some("active")
        || row.actor_role.as_deref() != Some("admin")
        || row.target_user_id == actor_id
        || row.target_role == "admin"
    {
        return Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied);
    }
    Ok(())
}

fn deterministic_image_key(prefix: &str, image: &LegacyImagePayloadV1) -> String {
    let extension = image
        .file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "image".into());
    format!("{prefix}/{}.{}", image.checksum_sha256, extension)
}

fn valid_r2_key(value: &str) -> bool {
    value.starts_with("organizations/")
        && value.len() <= 1024
        && !value.contains("..")
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
}

fn json_string_path(value: &str, path: &[&str]) -> AtomicResult<Option<String>> {
    let parsed: Value =
        serde_json::from_str(value).map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    let mut current = &parsed;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Ok(None);
        };
        current = next;
    }
    Ok(current.as_str().map(ToOwned::to_owned))
}

fn basic_effects(invalidation_paths: Vec<String>) -> LegacyOrganizationLibraryEffectsV1 {
    LegacyOrganizationLibraryEffectsV1 {
        invalidation_paths,
        set_verified_password_cookie: false,
        r2_keys_written: vec![],
        r2_keys_deleted: vec![],
    }
}

fn storage_effects(
    invalidation_paths: Vec<String>,
    puts: &[PutEffect],
    deletes: &[DeleteEffect],
) -> LegacyOrganizationLibraryEffectsV1 {
    LegacyOrganizationLibraryEffectsV1 {
        invalidation_paths,
        set_verified_password_cookie: false,
        r2_keys_written: puts
            .iter()
            .map(|effect| effect.object_key.clone())
            .collect(),
        r2_keys_deleted: deletes
            .iter()
            .map(|effect| effect.object_key.clone())
            .collect(),
    }
}

fn branding_paths() -> Vec<String> {
    vec![
        "/dashboard/caps".into(),
        "/dashboard/settings/organization".into(),
        "/dashboard/settings/organization/preferences".into(),
    ]
}

fn storage_paths() -> Vec<String> {
    vec![
        "/dashboard/settings/organization/integrations".into(),
        "/dashboard/settings/organization".into(),
    ]
}

fn organization_mutation_plan(
    organization_id: String,
    row: ActiveOrganizationRow,
    result: LegacyOrganizationLibraryResultV1,
    paths: Vec<String>,
    mutation: MutationStatement,
) -> ExecutionPlan {
    ExecutionPlan {
        organization_id,
        authority: AuthorityFence::Organization {
            row,
            mode: "manager",
        },
        result,
        effects: basic_effects(paths),
        mutations: vec![mutation],
        puts: vec![],
        deletes: vec![],
    }
}

fn preserve_pro_settings(submitted: &Value, existing_json: &str) -> AtomicResult<Value> {
    const PRO_KEYS: &[&str] = &[
        "disableSummary",
        "disableChapters",
        "disableTranscript",
        "hideShareableLinkCapLogo",
        "shareableLinkUseOrganizationIcon",
        "aiGenerationLanguage",
    ];
    let mut next = submitted
        .as_object()
        .cloned()
        .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    let existing: Value = serde_json::from_str(existing_json)
        .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    for key in PRO_KEYS {
        let default = match *key {
            "aiGenerationLanguage" => json!("auto"),
            _ => json!(false),
        };
        next.insert(
            (*key).into(),
            existing.get(*key).cloned().unwrap_or(default),
        );
    }
    Ok(Value::Object(next))
}

fn normalize_organization_settings(settings: &Value) -> AtomicResult<Value> {
    const SPEEDS: &[f64] = &[0.5, 0.75, 1.0, 1.2, 1.5, 1.75, 2.0];
    let mut object = settings
        .as_object()
        .cloned()
        .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
    if let Some(speed) = object.get("defaultPlaybackSpeed").and_then(Value::as_f64) {
        let mut closest = SPEEDS[0];
        let mut delta = (speed - closest).abs();
        for candidate in SPEEDS.iter().copied() {
            let next_delta = (speed - candidate).abs();
            if next_delta < delta {
                closest = candidate;
                delta = next_delta;
            }
        }
        let number = serde_json::Number::from_f64(closest)
            .ok_or(LegacyOrganizationLibraryAtomicErrorV1::Corrupt)?;
        object.insert("defaultPlaybackSpeed".into(), Value::Number(number));
    }
    Ok(Value::Object(object))
}

fn google_drive_location_configured(capabilities_json: &str) -> bool {
    serde_json::from_str::<Value>(capabilities_json)
        .ok()
        .is_some_and(|value| {
            value
                .get("folderId")
                .or_else(|| value.get("folder_id"))
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
                || value
                    .get("locationConfigured")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
}

fn storage_settings_json(
    legacy_organization_id: &str,
    organization_name: &str,
    rows: &[StorageRow],
    config: &LegacyOrganizationLibraryLocalConfigV1,
) -> AtomicResult<Value> {
    let active_provider = rows.iter().find(|row| row.state == "active").map(|row| {
        if row.provider == "google_drive" {
            "googleDrive"
        } else {
            "s3"
        }
    });
    let s3 = rows
        .iter()
        .find(|row| matches!(row.provider.as_str(), "r2" | "s3_compatible" | "minio"))
        .map(|row| {
            json!({
                "configured": row.state != "revoked",
                "provider": row.provider,
                "accessKeyId": "",
                "secretAccessKey": "",
                "endpoint": "",
                "bucketName": "",
                "region": "",
            })
        });
    let google_drive = rows.iter().find(|row| row.provider == "google_drive").map(|row| {
        let capabilities = serde_json::from_str::<Value>(&row.capabilities_json)
            .unwrap_or_else(|_| json!({}));
        json!({
            "id": row.id,
            "connected": row.state == "active" || row.state == "disabled",
            "active": row.state == "active",
            "status": if row.state == "active" { "active" } else { "disconnected" },
            "displayName": capabilities.get("displayName").cloned().unwrap_or(Value::Null),
            "email": capabilities.get("email").cloned().unwrap_or(Value::Null),
            "folderId": capabilities.get("folderId").or_else(|| capabilities.get("folder_id")).cloned().unwrap_or(Value::Null),
            "folderName": capabilities.get("folderName").or_else(|| capabilities.get("folder_name")).cloned().unwrap_or(Value::Null),
            "driveId": capabilities.get("driveId").or_else(|| capabilities.get("drive_id")).cloned().unwrap_or(Value::Null),
            "driveName": capabilities.get("driveName").or_else(|| capabilities.get("drive_name")).cloned().unwrap_or(Value::Null),
        })
    });
    serde_json::to_value(json!({
        "organization": {"id": legacy_organization_id, "name": organization_name},
        "activeProvider": active_provider,
        "googleOAuthClientId": if config.google_client_id.is_empty() { Value::Null } else { Value::String(config.google_client_id.clone()) },
        "googlePickerApiKey": config.google_picker_api_key.clone(),
        "s3": s3,
        "googleDrive": google_drive,
    }))
    .map_err(|_| LegacyOrganizationLibraryAtomicErrorV1::Corrupt)
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_password_hash_is_verified_without_disclosing_timing_comparison() {
        let salt = b"frame-test-salt!";
        let mut derived = [0_u8; 32];
        pbkdf2_hmac::<Sha256>(b"correct horse", salt, 100_000, &mut derived);
        let mut encoded = salt.to_vec();
        encoded.extend_from_slice(&derived);
        let stored = STANDARD.encode(encoded);
        assert!(verify_password(&stored, "correct horse"));
        assert!(!verify_password(&stored, "wrong"));
        assert!(!verify_password("malformed", "correct horse"));
    }

    #[test]
    fn playback_speed_uses_the_pinned_nearest_value_projection() {
        let normalized = normalize_organization_settings(&json!({
            "defaultPlaybackSpeed": 1.33,
            "disableComments": true
        }))
        .expect("normalized");
        assert_eq!(normalized["defaultPlaybackSpeed"], json!(1.2));
        assert_eq!(normalized["disableComments"], json!(true));
    }

    #[test]
    fn extracted_provider_keys_must_belong_to_the_organization_prefix() {
        assert!(valid_r2_key(
            "organizations/00000000-0000-7000-8000-000000000001/icon/a.png"
        ));
        assert!(!valid_r2_key("https://cdn.example.test/icon.png"));
        assert!(!valid_r2_key("organizations/../foreign/icon.png"));
    }

    #[test]
    fn oauth_query_values_are_rfc3986_encoded() {
        assert_eq!(percent_encode("a b+c/"), "a%20b%2Bc%2F");
    }

    fn member_row() -> MemberRow {
        MemberRow {
            legacy_member_id: "0123456789abcde".into(),
            organization_id: "00000000-0000-7000-8000-000000000101".into(),
            target_user_id: "00000000-0000-7000-8000-000000000003".into(),
            target_role: "member".into(),
            target_state: "active".into(),
            has_pro_seat: 0,
            target_revision: 1,
            target_authority_version: 1,
            owner_id: "00000000-0000-7000-8000-000000000001".into(),
            actor_role: Some("admin".into()),
            actor_state: Some("active".into()),
            target_email: "member@example.test".into(),
            owner_invite_quota: 3,
            owner_subscription_id: Some("sub_owner".into()),
            owner_subscription_status: Some("active".into()),
            actor_invite_quota: 5,
            actor_subscription_id: Some("sub_admin".into()),
            actor_subscription_status: Some("active".into()),
        }
    }

    #[test]
    fn pro_seat_provider_matches_cap_active_subscription_priority() {
        let mut row = member_row();
        assert_eq!(seat_provider(&row), (5, Some("sub_admin".into())));
        row.actor_subscription_status = Some("canceled".into());
        assert_eq!(seat_provider(&row), (3, Some("sub_owner".into())));
        row.owner_subscription_status = Some("canceled".into());
        assert_eq!(seat_provider(&row), (3, Some("sub_owner".into())));
        assert!(user_is_pro(None, Some("third_party")));
        assert!(!user_is_pro(Some("canceled"), None));
    }

    #[test]
    fn organization_role_hierarchy_matches_the_source_policy() {
        let row = member_row();
        let admin = "00000000-0000-7000-8000-000000000002";
        assert!(require_can_change_role(&row, admin, "admin").is_ok());
        assert!(require_can_change_role(&row, admin, "viewer").is_err());
        assert!(require_can_mutate_member(&row, admin, true).is_ok());

        let mut peer_admin = row;
        peer_admin.target_role = "admin".into();
        assert!(require_can_change_role(&peer_admin, admin, "member").is_err());
        assert!(require_can_mutate_member(&peer_admin, admin, true).is_err());
    }
}
