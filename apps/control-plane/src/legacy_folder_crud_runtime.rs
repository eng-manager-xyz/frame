//! Provider-free atomic D1 authority for the four source-pinned Cap folder
//! CRUD identities. Transport adapters are intentionally out of scope here.

use async_trait::async_trait;
use frame_application::{
    LegacyFolderColorV1, LegacyFolderCrudAtomicErrorV1, LegacyFolderCrudAtomicOutcomeV1,
    LegacyFolderCrudAtomicPortV1, LegacyFolderCrudCommandV1, LegacyFolderCrudMutationResultV1,
    LegacyFolderCrudMutationV1, LegacyFolderCrudSurfaceV1, LegacyFolderPublicPagePatchV1,
    LegacyFolderScopeV1, LegacyMappedParentPatchV1,
};
use frame_domain::LegacyCapNanoId;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_folder_crud/clock_now.sql");
const AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_crud/authority_snapshot.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_folder_crud/operation_by_key.sql");
const SCOPE_SNAPSHOT_SQL: &str = include_str!("../queries/legacy_folder_crud/scope_snapshot.sql");
const FOLDER_SNAPSHOT_SQL: &str = include_str!("../queries/legacy_folder_crud/folder_snapshot.sql");
const CYCLE_SNAPSHOT_SQL: &str = include_str!("../queries/legacy_folder_crud/cycle_snapshot.sql");
const DELETE_SUBTREE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_subtree_snapshot.sql");
const OPERATION_CLAIM_SQL: &str = include_str!("../queries/legacy_folder_crud/operation_claim.sql");
const AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_folder_crud/authority_assert.sql");
const SCOPE_ASSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/scope_assert.sql");
const FOLDER_ASSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/folder_assert.sql");
const CYCLE_ASSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/cycle_assert.sql");
const CREATE_INSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/create_insert.sql");
const UPDATE_APPLY_SQL: &str = include_str!("../queries/legacy_folder_crud/update_apply.sql");
const UPDATE_DESCENDANT_DEPTHS_SQL: &str =
    include_str!("../queries/legacy_folder_crud/update_descendant_depths.sql");
const DELETE_TARGETS_STAGE_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_targets_stage.sql");
const DELETE_REPARENT_PERSONAL_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_reparent_personal.sql");
const DELETE_REPARENT_ORGANIZATION_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_reparent_organization.sql");
const DELETE_REPARENT_SPACE_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_reparent_space.sql");
const DELETE_ROOT_SQL: &str = include_str!("../queries/legacy_folder_crud/delete_root.sql");
const CREATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_crud/create_postcondition.sql");
const UPDATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_crud/update_postcondition.sql");
const DELETE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_crud/delete_postcondition.sql");
const RECEIPT_INSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/receipt_insert.sql");
const EFFECT_INSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_folder_crud/audit_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_folder_crud/operation_complete.sql");
const DURABLE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_crud/durable_postcondition.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_folder_crud/assertion_cleanup.sql");

const AUTHORITY_SENTINEL: &str = "frame_legacy_folder_crud_authority_v1";
const TARGET_SENTINEL: &str = "frame_legacy_folder_crud_target_v1";
const PARENT_SENTINEL: &str = "frame_legacy_folder_crud_parent_v1";
const CYCLE_SENTINEL: &str = "frame_legacy_folder_crud_cycle_v1";
const SCOPE_SENTINEL: &str = "frame_legacy_folder_crud_scope_v1";
const MUTATION_SENTINEL: &str = "frame_legacy_folder_crud_mutation_v1";
const IMMUTABLE_SENTINEL: &str = "frame_legacy_folder_crud_evidence_immutable_v1";
const MAX_DELETE_FOLDERS: i64 = 100_000;
const MAX_FOLDER_DEPTH: i64 = 32;

type AtomicResult<T> = Result<T, LegacyFolderCrudAtomicErrorV1>;

pub(crate) struct D1LegacyFolderCrudAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyFolderCrudAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyFolderCrudAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyFolderCrudAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyFolderCrudAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

fn map_d1_message(message: &str) -> LegacyFolderCrudAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::StaleAuthority
    } else if message.contains(TARGET_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::TargetMissing
    } else if message.contains(PARENT_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::ParentMissing
    } else if message.contains(CYCLE_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::RecursiveDefinition
    } else if message.contains(SCOPE_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::ScopeConflict
    } else if message.contains(MUTATION_SENTINEL) || message.contains(IMMUTABLE_SENTINEL) {
        LegacyFolderCrudAtomicErrorV1::Corrupt
    } else {
        LegacyFolderCrudAtomicErrorV1::Unavailable
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthorityRow {
    selection_revision: i64,
    owner_id: String,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: String,
    membership_revision: i64,
    membership_authority_version: i64,
    owner_has_pro_seat: i64,
    owner_membership_revision: i64,
    owner_membership_authority_version: i64,
}

impl AuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        let revisions = [
            self.selection_revision,
            self.organization_revision,
            self.organization_authority_version,
            self.membership_revision,
            self.membership_authority_version,
            self.owner_membership_revision,
            self.owner_membership_authority_version,
        ];
        if revisions.into_iter().any(|value| value < 0)
            || !matches!(self.owner_has_pro_seat, 0 | 1)
            || !matches!(
                self.membership_role.as_str(),
                "owner" | "admin" | "member" | "viewer"
            )
            || Uuid::parse_str(&self.owner_id).is_err()
        {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn manages_organization(&self) -> bool {
        matches!(self.membership_role.as_str(), "owner" | "admin")
    }

    fn owner_is_pro(&self) -> bool {
        self.owner_has_pro_seat == 1
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    mutation_kind: Option<String>,
    folder_id: Option<String>,
    legacy_folder_id: Option<String>,
    name: Option<String>,
    color: Option<String>,
    affected_folder_count: Option<i64>,
    effect_count: i64,
    audit_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct ScopeRow {
    scope_kind: String,
    scope_id: Option<String>,
    scope_revision: i64,
    scope_authority_version: i64,
    scope_creator_id: String,
    actor_space_role: String,
    actor_space_membership_revision: i64,
}

impl ScopeRow {
    fn validate(&self, organization_id: &str) -> AtomicResult<()> {
        let valid = match self.scope_kind.as_str() {
            "personal" => {
                self.scope_id.is_none()
                    && self.scope_revision == -1
                    && self.scope_authority_version == -1
                    && self.scope_creator_id.is_empty()
                    && self.actor_space_role.is_empty()
                    && self.actor_space_membership_revision == -1
            }
            "organization" => {
                self.scope_id.as_deref() == Some(organization_id)
                    && self.scope_revision == -1
                    && self.scope_authority_version == -1
                    && self.scope_creator_id.is_empty()
                    && self.actor_space_role.is_empty()
                    && self.actor_space_membership_revision == -1
            }
            "space" => {
                self.scope_id
                    .as_deref()
                    .is_some_and(|value| Uuid::parse_str(value).is_ok())
                    && self.scope_revision >= 0
                    && self.scope_authority_version >= 0
                    && Uuid::parse_str(&self.scope_creator_id).is_ok()
                    && valid_space_membership(
                        &self.actor_space_role,
                        self.actor_space_membership_revision,
                    )
            }
            _ => false,
        };
        valid
            .then_some(())
            .ok_or(LegacyFolderCrudAtomicErrorV1::Corrupt)
    }

    fn managed_by(&self, actor_id: &str, authority: &AuthorityRow) -> bool {
        authority.manages_organization()
            || (self.scope_kind == "space"
                && (self.scope_creator_id == actor_id || self.actor_space_role == "manager"))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FolderRow {
    id: String,
    legacy_folder_id: Option<String>,
    organization_id: String,
    space_id: Option<String>,
    scope_kind: String,
    scope_id: Option<String>,
    parent_id: Option<String>,
    created_by_user_id: String,
    storage_name: String,
    legacy_name: Option<String>,
    name: String,
    color: String,
    is_public: i64,
    settings_json: String,
    revision: i64,
    tree_revision: i64,
    depth: i64,
    scope_revision: i64,
    scope_authority_version: i64,
    scope_creator_id: String,
    actor_space_role: String,
    actor_space_membership_revision: i64,
}

impl FolderRow {
    fn validate(&self, organization_id: &str) -> AtomicResult<()> {
        if self.organization_id != organization_id
            || Uuid::parse_str(&self.id).is_err()
            || Uuid::parse_str(&self.created_by_user_id).is_err()
            || self.storage_name.is_empty()
            || self.name.chars().count() > 255
            || !matches!(self.color.as_str(), "normal" | "blue" | "red" | "yellow")
            || !matches!(self.is_public, 0 | 1)
            || self.revision < 0
            || self.tree_revision < 0
            || !(0..=MAX_FOLDER_DEPTH).contains(&self.depth)
            || serde_json::from_str::<Value>(&self.settings_json).is_err()
            || self
                .legacy_folder_id
                .as_deref()
                .is_some_and(|value| LegacyCapNanoId::parse(value.to_owned()).is_err())
            || self
                .parent_id
                .as_deref()
                .is_some_and(|value| Uuid::parse_str(value).is_err())
        {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        ScopeRow {
            scope_kind: self.scope_kind.clone(),
            scope_id: self.scope_id.clone(),
            scope_revision: self.scope_revision,
            scope_authority_version: self.scope_authority_version,
            scope_creator_id: self.scope_creator_id.clone(),
            actor_space_role: self.actor_space_role.clone(),
            actor_space_membership_revision: self.actor_space_membership_revision,
        }
        .validate(organization_id)?;
        let storage_matches = match self.scope_kind.as_str() {
            "personal" | "organization" => self.space_id.is_none(),
            "space" => self.space_id == self.scope_id,
            _ => false,
        };
        storage_matches
            .then_some(())
            .ok_or(LegacyFolderCrudAtomicErrorV1::Corrupt)
    }

    fn same_scope(&self, other: &Self) -> bool {
        self.scope_kind == other.scope_kind && self.scope_id == other.scope_id
    }

    fn editable_by(&self, actor_id: &str, authority: &AuthorityRow) -> bool {
        match self.scope_kind.as_str() {
            "personal" => self.created_by_user_id == actor_id,
            "organization" => authority.manages_organization(),
            "space" => {
                authority.manages_organization()
                    || self.scope_creator_id == actor_id
                    || self.actor_space_role == "manager"
            }
            _ => false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CycleRow {
    cycle_count: i64,
    ancestor_count: i64,
}

#[derive(Debug, Deserialize)]
struct SubtreeRow {
    folder_count: i64,
    max_depth: i64,
    ids_json: String,
}

fn valid_space_membership(role: &str, revision: i64) -> bool {
    match role {
        "" => revision == -1,
        "manager" | "contributor" | "viewer" => revision >= 0,
        _ => false,
    }
}

impl D1LegacyFolderCrudAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.pop().expect("one clock row").now_ms;
        if !(0..=9_007_199_254_740_991).contains(&now_ms) {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        Ok(now_ms)
    }

    async fn authority(&self, actor_id: &str, organization_id: &str) -> AtomicResult<AuthorityRow> {
        let mut rows = self
            .rows::<AuthorityRow>(
                AUTHORITY_SNAPSHOT_SQL,
                vec![js(actor_id), js(organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderCrudAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }

    async fn operation(
        &self,
        organization_id: &str,
        actor_id: &str,
        source_operation_id: &str,
        key_digest: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(organization_id),
                    js(actor_id),
                    js(source_operation_id),
                    js(key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn scope(
        &self,
        organization_id: &str,
        actor_id: &str,
        kind: &str,
        scope_id: Option<&str>,
    ) -> AtomicResult<ScopeRow> {
        let mut rows = self
            .rows::<ScopeRow>(
                SCOPE_SNAPSHOT_SQL,
                vec![
                    js(organization_id),
                    js(actor_id),
                    js(kind),
                    js_opt(scope_id),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderCrudAtomicErrorV1::TargetMissing)?;
        row.validate(organization_id)?;
        Ok(row)
    }

    async fn folder(
        &self,
        folder_id: &str,
        organization_id: &str,
        actor_id: &str,
    ) -> AtomicResult<FolderRow> {
        let mut rows = self
            .rows::<FolderRow>(
                FOLDER_SNAPSHOT_SQL,
                vec![js(folder_id), js(organization_id), js(actor_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderCrudAtomicErrorV1::TargetMissing)?;
        row.validate(organization_id)?;
        Ok(row)
    }

    async fn cycle(
        &self,
        parent_id: &str,
        folder_id: &str,
        organization_id: &str,
    ) -> AtomicResult<CycleRow> {
        let mut rows = self
            .rows::<CycleRow>(
                CYCLE_SNAPSHOT_SQL,
                vec![js(parent_id), js(folder_id), js(organization_id)],
            )
            .await?;
        if rows.len() != 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let row = rows.pop().expect("one cycle row");
        if row.cycle_count < 0 || row.ancestor_count < 1 || row.ancestor_count > 33 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        Ok(row)
    }

    async fn subtree(&self, folder_id: &str, organization_id: &str) -> AtomicResult<SubtreeRow> {
        let mut rows = self
            .rows::<SubtreeRow>(
                DELETE_SUBTREE_SNAPSHOT_SQL,
                vec![js(folder_id), js(organization_id)],
            )
            .await?;
        if rows.len() != 1 {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let row = rows.pop().expect("one subtree row");
        if !(1..=MAX_DELETE_FOLDERS).contains(&row.folder_count)
            || !(0..=MAX_FOLDER_DEPTH).contains(&row.max_depth)
        {
            return Err(LegacyFolderCrudAtomicErrorV1::Conflict);
        }
        let ids: Vec<String> = serde_json::from_str(&row.ids_json)
            .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)?;
        if i64::try_from(ids.len()).ok() != Some(row.folder_count)
            || ids.windows(2).any(|pair| pair[0] >= pair[1])
            || ids.iter().any(|value| Uuid::parse_str(value).is_err())
            || !ids.iter().any(|value| value == folder_id)
        {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        Ok(row)
    }
}

impl D1LegacyFolderCrudAtomicPortV1<'_> {
    fn authority_assertion(
        &self,
        operation_id: &str,
        actor_id: &str,
        organization_id: &str,
        authority: &AuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(actor_id),
                js(organization_id),
                number(authority.selection_revision),
                js(&authority.owner_id),
                number(authority.organization_revision),
                number(authority.organization_authority_version),
                js(&authority.membership_role),
                number(authority.membership_revision),
                number(authority.membership_authority_version),
                number(authority.owner_has_pro_seat),
                number(authority.owner_membership_revision),
                number(authority.owner_membership_authority_version),
            ],
        )
    }

    fn scope_assertion(
        &self,
        operation_id: &str,
        organization_id: &str,
        actor_id: &str,
        scope: &ScopeRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            SCOPE_ASSERT_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                js(actor_id),
                js(&scope.scope_kind),
                js_opt(scope.scope_id.as_deref()),
                number(scope.scope_revision),
                number(scope.scope_authority_version),
                js(&scope.scope_creator_id),
                js(&scope.actor_space_role),
                number(scope.actor_space_membership_revision),
            ],
        )
    }

    fn folder_assertion(
        &self,
        operation_id: &str,
        kind: &str,
        organization_id: &str,
        actor_id: &str,
        folder: &FolderRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            FOLDER_ASSERT_SQL,
            vec![
                js(operation_id),
                js(kind),
                js(&folder.id),
                js(organization_id),
                js(actor_id),
                js_opt(folder.space_id.as_deref()),
                js(&folder.scope_kind),
                js_opt(folder.scope_id.as_deref()),
                js_opt(folder.parent_id.as_deref()),
                js(&folder.created_by_user_id),
                js(&folder.storage_name),
                js_opt(folder.legacy_name.as_deref()),
                js(&folder.name),
                js(&folder.color),
                number(folder.is_public),
                js(&folder.settings_json),
                number(folder.revision),
                number(folder.tree_revision),
                number(folder.depth),
                number(folder.scope_revision),
                number(folder.scope_authority_version),
                js(&folder.scope_creator_id),
                js(&folder.actor_space_role),
                number(folder.actor_space_membership_revision),
            ],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evidence_statements(
        &self,
        command: &LegacyFolderCrudCommandV1,
        operation_id: &str,
        action: &str,
        folder_id: &str,
        scope_kind: &str,
        scope_id: Option<&str>,
        receipt: &ReceiptShape<'_>,
        affected_count: i64,
        now_ms: i64,
    ) -> AtomicResult<Vec<D1PreparedStatement>> {
        let actor_id = command.authority().actor_id().to_string();
        let organization_id = command.authority().active_organization_id().to_string();
        let source_operation_id = command.surface().operation_id();
        let invalidation_json = serde_json::to_string(&json!({
            "paths": ["/dashboard/caps"],
            "folderIds": [folder_id],
            "scopeKind": scope_kind,
            "scopeId": scope_id,
        }))
        .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)?;
        let principal_digest =
            digest_fields(b"frame.legacy-folder-crud.principal.v1\0", &[&actor_id]);
        let mutation_digest = digest_fields(
            b"frame.legacy-folder-crud.subject.v1\0",
            &[
                &organization_id,
                source_operation_id,
                folder_id,
                &command.request_digest_hex(),
            ],
        );
        let audit_id = Uuid::now_v7().to_string();
        Ok(vec![
            self.statement(
                RECEIPT_INSERT_SQL,
                vec![
                    js(operation_id),
                    js(receipt.result_kind),
                    js(action),
                    js(folder_id),
                    js_opt(receipt.legacy_folder_id),
                    js_opt(receipt.name),
                    js_opt(receipt.color),
                    number(affected_count),
                    number(now_ms),
                ],
            )?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(action),
                    js(scope_kind),
                    js_opt(scope_id),
                    js(&invalidation_json),
                    number(affected_count),
                    number(now_ms),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    js(&audit_id),
                    js(operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(source_operation_id),
                    js(&principal_digest),
                    js(&mutation_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![js(operation_id), number(now_ms)],
            )?,
            self.statement(
                DURABLE_POSTCONDITION_SQL,
                vec![
                    js(operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(source_operation_id),
                    js(&command.request_digest_hex()),
                    number(now_ms),
                    js(action),
                    js(folder_id),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![js(operation_id)])?,
        ])
    }
}

struct ReceiptShape<'value> {
    result_kind: &'static str,
    legacy_folder_id: Option<&'value str>,
    name: Option<&'value str>,
    color: Option<&'value str>,
}

fn scope_parts(scope: LegacyFolderScopeV1, organization_id: &str) -> (String, Option<String>) {
    match scope {
        LegacyFolderScopeV1::Personal => ("personal".into(), None),
        LegacyFolderScopeV1::OrganizationLibrary => {
            ("organization".into(), Some(organization_id.into()))
        }
        LegacyFolderScopeV1::Space(space_id) => ("space".into(), Some(space_id.to_string())),
    }
}

fn public_page_json(patch: &LegacyFolderPublicPagePatchV1) -> AtomicResult<String> {
    let mut object = Map::new();
    if let Some(value) = patch.hide_title {
        object.insert("hideTitle".into(), Value::Bool(value));
    }
    if let Some(value) = patch.hide_copy_link {
        object.insert("hideCopyLink".into(), Value::Bool(value));
    }
    if let Some(value) = patch.logo_mode {
        object.insert("logoMode".into(), Value::String(value.as_str().into()));
    }
    if let Some(value) = &patch.title {
        object.insert("title".into(), Value::String(value.clone()));
    }
    if let Some(value) = &patch.subtitle {
        object.insert("subtitle".into(), Value::String(value.clone()));
    }
    if let Some(value) = &patch.cta_label {
        object.insert("ctaLabel".into(), Value::String(value.clone()));
    }
    if let Some(value) = &patch.cta_url {
        object.insert("ctaUrl".into(), Value::String(value.clone()));
    }
    if let Some(value) = patch.layout {
        object.insert("layout".into(), Value::String(value.as_str().into()));
    }
    if let Some(value) = patch.grid_columns {
        object.insert("gridColumns".into(), Value::from(value));
    }
    serde_json::to_string(&Value::Object(object))
        .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)
}

impl D1LegacyFolderCrudAtomicPortV1<'_> {
    async fn execute_create(
        &self,
        command: &LegacyFolderCrudCommandV1,
        authority: &AuthorityRow,
    ) -> AtomicResult<LegacyFolderCrudMutationResultV1> {
        let LegacyFolderCrudMutationV1::Create {
            legacy_folder_id,
            folder_id,
            name,
            color,
            is_public,
            scope,
            parent_id,
        } = command.mutation()
        else {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        };
        let actor_id = command.authority().actor_id().to_string();
        let organization_id = command.authority().active_organization_id().to_string();
        let (scope_kind, scope_id) = scope_parts(*scope, &organization_id);
        let scope_snapshot = self
            .scope(
                &organization_id,
                &actor_id,
                &scope_kind,
                scope_id.as_deref(),
            )
            .await?;
        if scope_kind != "personal" && !scope_snapshot.managed_by(&actor_id, authority) {
            return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
        }
        if *is_public && !authority.owner_is_pro() {
            return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
        }
        let parent = match parent_id {
            Some(parent_id) => {
                let row = self
                    .folder(&parent_id.to_string(), &organization_id, &actor_id)
                    .await
                    .map_err(|error| match error {
                        LegacyFolderCrudAtomicErrorV1::TargetMissing => {
                            LegacyFolderCrudAtomicErrorV1::ParentMissing
                        }
                        error => error,
                    })?;
                if !row.editable_by(&actor_id, authority) {
                    return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
                }
                if row.scope_kind != scope_kind || row.scope_id != scope_id {
                    return Err(LegacyFolderCrudAtomicErrorV1::ScopeConflict);
                }
                Some(row)
            }
            None => None,
        };
        let depth = parent.as_ref().map_or(0, |row| row.depth + 1);
        if depth > MAX_FOLDER_DEPTH {
            return Err(LegacyFolderCrudAtomicErrorV1::ScopeConflict);
        }
        let operation_id = command.operation_id().to_string();
        let now_ms = self.clock_now().await?;
        let folder_id = folder_id.to_string();
        let legacy_folder_id = legacy_folder_id.as_str();
        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(command.surface().operation_id()),
                    js(&command.idempotency_key_digest_hex()),
                    js(&command.request_digest_hex()),
                    number(now_ms),
                ],
            )?,
            self.authority_assertion(&operation_id, &actor_id, &organization_id, authority)?,
            self.scope_assertion(&operation_id, &organization_id, &actor_id, &scope_snapshot)?,
        ];
        if let Some(parent) = &parent {
            statements.push(self.folder_assertion(
                &operation_id,
                "parent",
                &organization_id,
                &actor_id,
                parent,
            )?);
        }
        statements.extend([
            self.statement(
                CREATE_INSERT_SQL,
                vec![
                    js(&folder_id),
                    js(legacy_folder_id),
                    js(&organization_id),
                    js_opt(
                        scope_snapshot
                            .scope_id
                            .as_deref()
                            .filter(|_| scope_kind == "space"),
                    ),
                    js_opt(parent.as_ref().map(|row| row.id.as_str())),
                    js(&actor_id),
                    js(name),
                    js(color.as_str()),
                    number(i64::from(*is_public)),
                    number(now_ms),
                    number(depth),
                    js(&operation_id),
                    js(&scope_kind),
                    js_opt(scope_id.as_deref()),
                ],
            )?,
            self.statement(
                CREATE_POSTCONDITION_SQL,
                vec![
                    js(&operation_id),
                    js(&folder_id),
                    js(legacy_folder_id),
                    js(&organization_id),
                    js_opt(
                        scope_snapshot
                            .scope_id
                            .as_deref()
                            .filter(|_| scope_kind == "space"),
                    ),
                    js_opt(parent.as_ref().map(|row| row.id.as_str())),
                    js(&actor_id),
                    js(name),
                    js(color.as_str()),
                    number(i64::from(*is_public)),
                    number(depth),
                    js(&scope_kind),
                    js_opt(scope_id.as_deref()),
                ],
            )?,
        ]);
        let receipt = if command.surface() == LegacyFolderCrudSurfaceV1::MobileCreate {
            ReceiptShape {
                result_kind: "mobile_created",
                legacy_folder_id: Some(legacy_folder_id),
                name: Some(name),
                color: Some(color.as_str()),
            }
        } else {
            ReceiptShape {
                result_kind: "rpc_void",
                legacy_folder_id: None,
                name: None,
                color: None,
            }
        };
        statements.extend(self.evidence_statements(
            command,
            &operation_id,
            "create",
            &folder_id,
            &scope_kind,
            scope_id.as_deref(),
            &receipt,
            1,
            now_ms,
        )?);
        self.batch(statements).await?;
        if command.surface() == LegacyFolderCrudSurfaceV1::MobileCreate {
            Ok(LegacyFolderCrudMutationResultV1::MobileCreated {
                legacy_folder_id: LegacyCapNanoId::parse(legacy_folder_id.to_owned())
                    .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)?,
                name: name.clone(),
                color: *color,
            })
        } else {
            Ok(LegacyFolderCrudMutationResultV1::RpcVoid)
        }
    }

    async fn execute_update(
        &self,
        command: &LegacyFolderCrudCommandV1,
        authority: &AuthorityRow,
    ) -> AtomicResult<LegacyFolderCrudMutationResultV1> {
        let LegacyFolderCrudMutationV1::Update {
            folder_id,
            name,
            color,
            is_public,
            public_page,
            parent_id,
        } = command.mutation()
        else {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        };
        let actor_id = command.authority().actor_id().to_string();
        let organization_id = command.authority().active_organization_id().to_string();
        let folder_id = folder_id.to_string();
        let folder = self.folder(&folder_id, &organization_id, &actor_id).await?;
        if !folder.editable_by(&actor_id, authority) {
            return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
        }
        if (is_public == &Some(true) || public_page.is_some()) && !authority.owner_is_pro() {
            return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
        }
        let mut parent = None;
        let (parent_mode, final_parent_id, final_depth) = match parent_id {
            LegacyMappedParentPatchV1::Absent => ("absent", folder.parent_id.clone(), folder.depth),
            LegacyMappedParentPatchV1::Root => ("root", None, 0),
            LegacyMappedParentPatchV1::Parent(parent_id) => {
                if parent_id.to_string() == folder_id {
                    return Err(LegacyFolderCrudAtomicErrorV1::RecursiveDefinition);
                }
                let row = self
                    .folder(&parent_id.to_string(), &organization_id, &actor_id)
                    .await
                    .map_err(|error| match error {
                        LegacyFolderCrudAtomicErrorV1::TargetMissing => {
                            LegacyFolderCrudAtomicErrorV1::ParentMissing
                        }
                        error => error,
                    })?;
                if !row.editable_by(&actor_id, authority) {
                    return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
                }
                if !folder.same_scope(&row) {
                    return Err(LegacyFolderCrudAtomicErrorV1::ScopeConflict);
                }
                let cycle = self.cycle(&row.id, &folder.id, &organization_id).await?;
                if cycle.cycle_count != 0 {
                    return Err(LegacyFolderCrudAtomicErrorV1::RecursiveDefinition);
                }
                let depth = row.depth + 1;
                parent = Some(row);
                ("parent", Some(parent_id.to_string()), depth)
            }
        };
        let parent_changed = !matches!(parent_id, LegacyMappedParentPatchV1::Absent);
        if parent_changed {
            let subtree = self.subtree(&folder.id, &organization_id).await?;
            if final_depth + subtree.max_depth > MAX_FOLDER_DEPTH {
                return Err(LegacyFolderCrudAtomicErrorV1::ScopeConflict);
            }
        }
        let has_change = name.is_some()
            || color.is_some()
            || is_public.is_some()
            || public_page.is_some()
            || parent_changed;
        let patch_json = public_page
            .as_ref()
            .map(public_page_json)
            .transpose()?
            .unwrap_or_else(|| "{}".into());
        let expected_name = name.as_deref().unwrap_or(&folder.name);
        let expected_storage_name = match name.as_deref() {
            Some("") => "\u{2063}",
            Some(value) => value,
            None => folder.storage_name.as_str(),
        };
        let expected_legacy_name = match name {
            Some(value) => Some(value.as_str()),
            None => folder.legacy_name.as_deref(),
        };
        let expected_color =
            color.map_or_else(|| folder.color.clone(), |value| value.as_str().to_owned());
        let expected_public = is_public.map_or(folder.is_public, i64::from);
        let expected_revision = folder.revision + i64::from(has_change);
        let expected_tree_revision = folder.tree_revision + i64::from(parent_changed);
        let operation_id = command.operation_id().to_string();
        let now_ms = self.clock_now().await?;
        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(command.surface().operation_id()),
                    js(&command.idempotency_key_digest_hex()),
                    js(&command.request_digest_hex()),
                    number(now_ms),
                ],
            )?,
            self.authority_assertion(&operation_id, &actor_id, &organization_id, authority)?,
            self.folder_assertion(
                &operation_id,
                "target",
                &organization_id,
                &actor_id,
                &folder,
            )?,
        ];
        if let Some(parent) = &parent {
            statements.push(self.folder_assertion(
                &operation_id,
                "parent",
                &organization_id,
                &actor_id,
                parent,
            )?);
            statements.push(self.statement(
                CYCLE_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&parent.id),
                    js(&folder.id),
                    js(&organization_id),
                ],
            )?);
        }
        if has_change {
            statements.push(self.statement(
                UPDATE_APPLY_SQL,
                vec![
                    js(&folder.id),
                    number(i64::from(name.is_some())),
                    js(name.as_deref().unwrap_or("")),
                    number(i64::from(color.is_some())),
                    js(color.map_or("normal", LegacyFolderColorV1::as_str)),
                    number(i64::from(is_public.is_some())),
                    number(is_public.map_or(0, i64::from)),
                    number(i64::from(public_page.is_some())),
                    js(&patch_json),
                    js(parent_mode),
                    js_opt(final_parent_id.as_deref()),
                    number(final_depth),
                    number(1),
                    number(now_ms),
                    js(&operation_id),
                    js(&organization_id),
                    number(folder.revision),
                    number(folder.tree_revision),
                ],
            )?);
            if parent_changed && final_depth != folder.depth {
                statements.push(self.statement(
                    UPDATE_DESCENDANT_DEPTHS_SQL,
                    vec![
                        js(&folder.id),
                        js(&organization_id),
                        number(final_depth - folder.depth),
                        number(now_ms),
                        js(&operation_id),
                    ],
                )?);
            }
        }
        statements.push(self.statement(
            UPDATE_POSTCONDITION_SQL,
            vec![
                js(&operation_id),
                js(&folder.id),
                js(&organization_id),
                js(expected_name),
                js(expected_storage_name),
                js_opt(expected_legacy_name),
                js(&expected_color),
                number(expected_public),
                number(i64::from(public_page.is_some())),
                js(&folder.settings_json),
                js(&patch_json),
                js_opt(final_parent_id.as_deref()),
                number(expected_revision),
                number(expected_tree_revision),
                number(final_depth),
                number(i64::from(has_change)),
            ],
        )?);
        let receipt = ReceiptShape {
            result_kind: "rpc_void",
            legacy_folder_id: None,
            name: None,
            color: None,
        };
        statements.extend(self.evidence_statements(
            command,
            &operation_id,
            "update",
            &folder.id,
            &folder.scope_kind,
            folder.scope_id.as_deref(),
            &receipt,
            i64::from(has_change),
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyFolderCrudMutationResultV1::RpcVoid)
    }

    async fn execute_delete(
        &self,
        command: &LegacyFolderCrudCommandV1,
        authority: &AuthorityRow,
    ) -> AtomicResult<LegacyFolderCrudMutationResultV1> {
        let LegacyFolderCrudMutationV1::Delete { folder_id } = command.mutation() else {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        };
        let actor_id = command.authority().actor_id().to_string();
        let organization_id = command.authority().active_organization_id().to_string();
        let folder = self
            .folder(&folder_id.to_string(), &organization_id, &actor_id)
            .await?;
        if !folder.editable_by(&actor_id, authority) {
            return Err(LegacyFolderCrudAtomicErrorV1::AccessDenied);
        }
        let subtree = self.subtree(&folder.id, &organization_id).await?;
        let operation_id = command.operation_id().to_string();
        let now_ms = self.clock_now().await?;
        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js(&actor_id),
                    js(command.surface().operation_id()),
                    js(&command.idempotency_key_digest_hex()),
                    js(&command.request_digest_hex()),
                    number(now_ms),
                ],
            )?,
            self.authority_assertion(&operation_id, &actor_id, &organization_id, authority)?,
            self.folder_assertion(
                &operation_id,
                "target",
                &organization_id,
                &actor_id,
                &folder,
            )?,
            self.statement(
                DELETE_TARGETS_STAGE_SQL,
                vec![js(&operation_id), js(&folder.id), js(&organization_id)],
            )?,
        ];
        match folder.scope_kind.as_str() {
            "personal" => statements.push(self.statement(
                DELETE_REPARENT_PERSONAL_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js_opt(folder.parent_id.as_deref()),
                    number(now_ms),
                ],
            )?),
            "organization" => statements.push(self.statement(
                DELETE_REPARENT_ORGANIZATION_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js_opt(folder.parent_id.as_deref()),
                    js(&operation_id),
                ],
            )?),
            "space" => statements.push(self.statement(
                DELETE_REPARENT_SPACE_SQL,
                vec![
                    js(&operation_id),
                    js(&organization_id),
                    js(folder.scope_id.as_deref().ok_or(
                        LegacyFolderCrudAtomicErrorV1::Corrupt,
                    )?),
                    js_opt(folder.parent_id.as_deref()),
                    js(&operation_id),
                ],
            )?),
            _ => return Err(LegacyFolderCrudAtomicErrorV1::Corrupt),
        }
        statements.extend([
            self.statement(
                DELETE_ROOT_SQL,
                vec![
                    js(&folder.id),
                    js(&organization_id),
                    number(folder.revision),
                    number(folder.tree_revision),
                ],
            )?,
            self.statement(
                DELETE_POSTCONDITION_SQL,
                vec![
                    js(&operation_id),
                    number(subtree.folder_count),
                    js(&subtree.ids_json),
                ],
            )?,
        ]);
        let receipt = ReceiptShape {
            result_kind: "rpc_void",
            legacy_folder_id: None,
            name: None,
            color: None,
        };
        statements.extend(self.evidence_statements(
            command,
            &operation_id,
            "delete",
            &folder.id,
            &folder.scope_kind,
            folder.scope_id.as_deref(),
            &receipt,
            subtree.folder_count,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyFolderCrudMutationResultV1::RpcVoid)
    }
}

impl D1LegacyFolderCrudAtomicPortV1<'_> {
    fn replay(
        &self,
        command: &LegacyFolderCrudCommandV1,
        operation: &OperationRow,
    ) -> AtomicResult<LegacyFolderCrudAtomicOutcomeV1> {
        if Uuid::parse_str(&operation.operation_id).is_err() {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        if operation.state == "claimed" {
            let evidence_absent = operation.result_kind.is_none()
                && operation.mutation_kind.is_none()
                && operation.folder_id.is_none()
                && operation.legacy_folder_id.is_none()
                && operation.name.is_none()
                && operation.color.is_none()
                && operation.affected_folder_count.is_none()
                && operation.effect_count == 0
                && operation.audit_count == 0;
            return if evidence_absent {
                Err(LegacyFolderCrudAtomicErrorV1::InFlight)
            } else {
                Err(LegacyFolderCrudAtomicErrorV1::Corrupt)
            };
        }
        if operation.state != "complete"
            || operation.effect_count != 1
            || operation.audit_count != 1
            || operation
                .affected_folder_count
                .is_none_or(|count| !(0..=MAX_DELETE_FOLDERS).contains(&count))
        {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let expected_folder_id = match command.mutation() {
            LegacyFolderCrudMutationV1::Create { folder_id, .. }
            | LegacyFolderCrudMutationV1::Delete { folder_id }
            | LegacyFolderCrudMutationV1::Update { folder_id, .. } => folder_id.to_string(),
        };
        if operation.folder_id.as_deref() != Some(expected_folder_id.as_str()) {
            return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
        }
        let result = match command.surface() {
            LegacyFolderCrudSurfaceV1::MobileCreate => {
                if operation.result_kind.as_deref() != Some("mobile_created")
                    || operation.mutation_kind.as_deref() != Some("create")
                    || operation.affected_folder_count != Some(1)
                {
                    return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
                }
                let legacy_folder_id = operation
                    .legacy_folder_id
                    .clone()
                    .ok_or(LegacyFolderCrudAtomicErrorV1::Corrupt)
                    .and_then(|value| {
                        LegacyCapNanoId::parse(value)
                            .map_err(|_| LegacyFolderCrudAtomicErrorV1::Corrupt)
                    })?;
                let color = parse_color(
                    operation
                        .color
                        .as_deref()
                        .ok_or(LegacyFolderCrudAtomicErrorV1::Corrupt)?,
                )?;
                LegacyFolderCrudMutationResultV1::MobileCreated {
                    legacy_folder_id,
                    name: operation
                        .name
                        .clone()
                        .ok_or(LegacyFolderCrudAtomicErrorV1::Corrupt)?,
                    color,
                }
            }
            surface => {
                let expected_action = match surface {
                    LegacyFolderCrudSurfaceV1::RpcCreate => "create",
                    LegacyFolderCrudSurfaceV1::RpcDelete => "delete",
                    LegacyFolderCrudSurfaceV1::RpcUpdate => "update",
                    LegacyFolderCrudSurfaceV1::MobileCreate => unreachable!(),
                };
                if operation.result_kind.as_deref() != Some("rpc_void")
                    || operation.mutation_kind.as_deref() != Some(expected_action)
                    || operation.legacy_folder_id.is_some()
                    || operation.name.is_some()
                    || operation.color.is_some()
                {
                    return Err(LegacyFolderCrudAtomicErrorV1::Corrupt);
                }
                LegacyFolderCrudMutationResultV1::RpcVoid
            }
        };
        Ok(LegacyFolderCrudAtomicOutcomeV1 {
            result,
            replayed: true,
        })
    }

    async fn reconcile(
        &self,
        command: &LegacyFolderCrudCommandV1,
        original_error: LegacyFolderCrudAtomicErrorV1,
    ) -> AtomicResult<LegacyFolderCrudAtomicOutcomeV1> {
        let organization_id = command.authority().active_organization_id().to_string();
        let actor_id = command.authority().actor_id().to_string();
        match self
            .operation(
                &organization_id,
                &actor_id,
                command.surface().operation_id(),
                &command.idempotency_key_digest_hex(),
            )
            .await
        {
            Ok(Some(operation)) if operation.request_digest == command.request_digest_hex() => {
                self.replay(command, &operation)
            }
            Ok(Some(_)) => Err(LegacyFolderCrudAtomicErrorV1::IdempotencyConflict),
            Ok(None) => Err(original_error),
            Err(_) => Err(LegacyFolderCrudAtomicErrorV1::Unavailable),
        }
    }
}

#[async_trait]
impl LegacyFolderCrudAtomicPortV1 for D1LegacyFolderCrudAtomicPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyFolderCrudCommandV1,
    ) -> AtomicResult<LegacyFolderCrudAtomicOutcomeV1> {
        let organization_id = command.authority().active_organization_id().to_string();
        let actor_id = command.authority().actor_id().to_string();
        let authority = self.authority(&actor_id, &organization_id).await?;
        match self
            .operation(
                &organization_id,
                &actor_id,
                command.surface().operation_id(),
                &command.idempotency_key_digest_hex(),
            )
            .await?
        {
            Some(operation) if operation.request_digest == command.request_digest_hex() => {
                return self.replay(&command, &operation);
            }
            Some(_) => return Err(LegacyFolderCrudAtomicErrorV1::IdempotencyConflict),
            None => {}
        }
        let fresh = match command.mutation() {
            LegacyFolderCrudMutationV1::Create { .. } => {
                self.execute_create(&command, &authority).await
            }
            LegacyFolderCrudMutationV1::Update { .. } => {
                self.execute_update(&command, &authority).await
            }
            LegacyFolderCrudMutationV1::Delete { .. } => {
                self.execute_delete(&command, &authority).await
            }
        };
        match fresh {
            Ok(result) => Ok(LegacyFolderCrudAtomicOutcomeV1 {
                result,
                replayed: false,
            }),
            Err(error) => self.reconcile(&command, error).await,
        }
    }
}

fn parse_color(value: &str) -> AtomicResult<LegacyFolderColorV1> {
    match value {
        "normal" => Ok(LegacyFolderColorV1::Normal),
        "blue" => Ok(LegacyFolderColorV1::Blue),
        "red" => Ok(LegacyFolderColorV1::Red),
        "yellow" => Ok(LegacyFolderColorV1::Yellow),
        _ => Err(LegacyFolderCrudAtomicErrorV1::Corrupt),
    }
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn js_opt(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn digest_fields(domain: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d1_messages_map_to_stable_failure_classes() {
        assert_eq!(
            map_d1_message(AUTHORITY_SENTINEL),
            LegacyFolderCrudAtomicErrorV1::StaleAuthority
        );
        assert_eq!(
            map_d1_message(TARGET_SENTINEL),
            LegacyFolderCrudAtomicErrorV1::TargetMissing
        );
        assert_eq!(
            map_d1_message(CYCLE_SENTINEL),
            LegacyFolderCrudAtomicErrorV1::RecursiveDefinition
        );
        assert_eq!(
            map_d1_message(PARENT_SENTINEL),
            LegacyFolderCrudAtomicErrorV1::ParentMissing
        );
        assert_eq!(
            map_d1_message(SCOPE_SENTINEL),
            LegacyFolderCrudAtomicErrorV1::ScopeConflict
        );
        assert_eq!(
            map_d1_message("provider detail"),
            LegacyFolderCrudAtomicErrorV1::Unavailable
        );
    }

    #[test]
    fn public_page_patch_omits_absent_fields_and_preserves_false() {
        let value = public_page_json(&LegacyFolderPublicPagePatchV1 {
            hide_title: Some(false),
            title: Some("Launches".into()),
            ..LegacyFolderPublicPagePatchV1::default()
        })
        .expect("patch");
        assert_eq!(value, r#"{"hideTitle":false,"title":"Launches"}"#);
    }

    #[test]
    fn folder_edit_policy_keeps_personal_creator_only() {
        let authority = AuthorityRow {
            selection_revision: 0,
            owner_id: Uuid::now_v7().to_string(),
            organization_revision: 0,
            organization_authority_version: 0,
            membership_role: "admin".into(),
            membership_revision: 0,
            membership_authority_version: 0,
            owner_has_pro_seat: 1,
            owner_membership_revision: 0,
            owner_membership_authority_version: 0,
        };
        let actor = Uuid::now_v7().to_string();
        let folder = FolderRow {
            id: Uuid::now_v7().to_string(),
            legacy_folder_id: None,
            organization_id: Uuid::now_v7().to_string(),
            space_id: None,
            scope_kind: "personal".into(),
            scope_id: None,
            parent_id: None,
            created_by_user_id: Uuid::now_v7().to_string(),
            storage_name: "Folder".into(),
            legacy_name: None,
            name: "Folder".into(),
            color: "normal".into(),
            is_public: 0,
            settings_json: "{}".into(),
            revision: 0,
            tree_revision: 0,
            depth: 0,
            scope_revision: -1,
            scope_authority_version: -1,
            scope_creator_id: String::new(),
            actor_space_role: String::new(),
            actor_space_membership_revision: -1,
        };
        assert!(!folder.editable_by(&actor, &authority));
    }

    #[test]
    fn query_bundle_contains_atomic_journal_and_recursive_delete() {
        assert!(OPERATION_CLAIM_SQL.contains("legacy_folder_crud_operations_v1"));
        assert!(DELETE_TARGETS_STAGE_SQL.contains("WITH RECURSIVE"));
        assert!(DELETE_POSTCONDITION_SQL.contains("legacy_folder_crud_delete_targets_v1"));
        assert!(DURABLE_POSTCONDITION_SQL.contains("legacy_folder_crud_audit_events_v1"));
    }
}
