//! Atomic D1 adapter for Cap's add/remove/move folder actions.
//!
//! The legacy handlers performed globally-addressed reads and split writes.
//! This adapter deliberately does neither: it snapshots bounded tenant rows,
//! reasserts the snapshot and one-use browser grant inside one D1 batch, then
//! proves the final assignment, durable receipt, invalidations, and audit.

use async_trait::async_trait;
use frame_application::{
    LegacyFolderAssignmentAtomicErrorV1, LegacyFolderAssignmentAtomicOutcomeV1,
    LegacyFolderAssignmentAtomicPortV1, LegacyFolderAssignmentAuthorizedContextV1,
    LegacyFolderAssignmentBrowserFenceV1, LegacyFolderAssignmentCommandV1,
    LegacyFolderAssignmentEffectsV1, LegacyFolderAssignmentInvalidationTargetV1,
    LegacyFolderAssignmentMutationReceiptV1, LegacyFolderAssignmentScopeV1,
};
use frame_domain::{FolderId, SpaceId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, send::IntoSendFuture};

const AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/authority_snapshot.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/operation_by_key.sql");
const FOLDER_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/folder_snapshot.sql");
const SPACE_CONTEXT_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/space_context_snapshot.sql");
const VIDEO_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/video_snapshot.sql");
const SPACE_ASSIGNMENT_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/space_assignment_snapshot.sql");
const SHARED_ASSIGNMENT_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/shared_assignment_snapshot.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/operation_claim.sql");
const ORGANIZATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/organization_assert.sql");
const SELECTION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/selection_assert.sql");
const MEMBERSHIP_ASSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/membership_assert.sql");
const PRODUCT_PRECONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/product_precondition.sql");
const PRODUCT_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/product_postcondition.sql");
const DIRECT_ASSIGNMENT_SET_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/direct_assignment_set.sql");
const SPACE_ASSIGNMENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/space_assignment_insert.sql");
const SPACE_ASSIGNMENT_SET_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/space_assignment_set.sql");
const SHARED_ASSIGNMENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/shared_assignment_insert.sql");
const SHARED_ASSIGNMENT_SET_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/shared_assignment_set.sql");
const SHARED_ASSIGNMENT_REVOKE_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/shared_assignment_revoke.sql");
const SHARED_ASSIGNMENT_REACTIVATE_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/shared_assignment_reactivate.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/effect_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/operation_complete.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_folder_assignment/audit_insert.sql");
const RECEIPT_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/receipt_postcondition.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_folder_assignment/assertion_cleanup.sql");
const BROWSER_MUTATION_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_assert.sql");
const BROWSER_MUTATION_GRANT_DELETE_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_delete_by_proof.sql");
const BROWSER_MUTATION_CHANGE_ASSERT_SQL: &str =
    include_str!("../queries/auth/browser_mutation_change_assert.sql");

const MAX_VIDEO_COUNT: usize = 500;
const AUDIT_ACTION: &str = "legacy.folder_assignment";

type AtomicResult<T> = Result<T, LegacyFolderAssignmentAtomicErrorV1>;

pub(crate) struct D1LegacyFolderAssignmentAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyFolderAssignmentAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Unavailable)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Unavailable);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AuthorityRow {
    selection_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: String,
    membership_revision: i64,
    membership_authority_version: i64,
}

impl AuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        let nonnegative = [
            self.selection_revision,
            self.organization_revision,
            self.organization_authority_version,
            self.membership_revision,
            self.membership_authority_version,
        ]
        .into_iter()
        .all(|value| value >= 0);
        if !nonnegative
            || !matches!(
                self.membership_role.as_str(),
                "owner" | "admin" | "member" | "viewer"
            )
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    response_json: Option<String>,
    effect_json: Option<String>,
    audit_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct FolderRow {
    id: String,
    space_id: Option<String>,
    parent_id: Option<String>,
    created_by_user_id: String,
    revision: i64,
    tree_revision: i64,
    space_revision: i64,
    space_authority_version: i64,
    actor_space_role: String,
    actor_space_membership_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct SpaceContextRow {
    id: String,
    revision: i64,
    authority_version: i64,
    actor_space_role: String,
    actor_space_membership_revision: i64,
}

#[derive(Debug, Deserialize)]
struct VideoRow {
    requested_id: String,
    id: Option<String>,
    owner_id: Option<String>,
    folder_id: Option<String>,
    revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct VideoSnapshot {
    id: String,
    owner_id: String,
    folder_id: Option<String>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct SpaceAssignmentRow {
    requested_id: String,
    video_id: Option<String>,
    folder_id: Option<String>,
    revision: Option<i64>,
}

#[derive(Debug, Clone)]
struct SpaceAssignmentSnapshot {
    id: String,
    present: bool,
    folder_id: Option<String>,
    revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct SharedAssignmentSnapshot {
    requested_id: String,
    active_count: i64,
    active_id: String,
    active_folder_id: Option<String>,
    active_revision: i64,
    dormant_id: String,
    dormant_revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Add,
    Remove,
    Move,
}

impl Action {
    const fn journal_name(self) -> &'static str {
        match self {
            Self::Add => "legacy_folder_assignment_add_v1",
            Self::Remove => "legacy_folder_assignment_remove_v1",
            Self::Move => "legacy_folder_assignment_move_v1",
        }
    }

    const fn policy_name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Move => "move",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StorageContext {
    Direct,
    Organization,
    Space(String),
}

impl StorageContext {
    const fn label(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Organization => "organization",
            Self::Space(_) => "space",
        }
    }

    fn space_id(&self) -> Option<&str> {
        match self {
            Self::Space(space_id) => Some(space_id),
            Self::Direct | Self::Organization => None,
        }
    }
}

#[derive(Debug, Clone)]
struct CommandShape {
    action: Action,
    folder_id: Option<String>,
    video_ids: Vec<String>,
    context: StorageContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectWire {
    invalidates_caps: bool,
    targets: Vec<EffectTargetWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
enum EffectTargetWire {
    Folder { folder_id: String },
    SpaceRoot { space_id: String },
    SpaceFolder { space_id: String, folder_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextWire {
    target_folder_space_id: Option<String>,
    original_folder_id: Option<String>,
    original_parent_id: Option<String>,
    target_parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptWire {
    affected_count: Option<u16>,
    effects: EffectWire,
    authorized_context: ContextWire,
}

impl CommandShape {
    fn from_command(command: &LegacyFolderAssignmentCommandV1) -> AtomicResult<Self> {
        let organization_id = command
            .fence()
            .authority()
            .active_organization_id()
            .to_string();
        let shape = match command {
            LegacyFolderAssignmentCommandV1::Add {
                folder_id,
                video_ids,
                scope,
                ..
            } => Self {
                action: Action::Add,
                folder_id: Some(folder_id.to_string()),
                video_ids: video_ids.iter().map(ToString::to_string).collect(),
                context: context_from_scope(*scope, &organization_id)?,
            },
            LegacyFolderAssignmentCommandV1::Remove {
                folder_id,
                video_ids,
                scope,
                ..
            } => Self {
                action: Action::Remove,
                folder_id: Some(folder_id.to_string()),
                video_ids: video_ids.iter().map(ToString::to_string).collect(),
                context: context_from_scope(*scope, &organization_id)?,
            },
            LegacyFolderAssignmentCommandV1::Move {
                video_id,
                folder_id,
                scope,
                ..
            } => Self {
                action: Action::Move,
                folder_id: folder_id.map(|folder_id| folder_id.to_string()),
                video_ids: vec![video_id.to_string()],
                context: match scope {
                    Some(scope) => context_from_scope(*scope, &organization_id)?,
                    None => StorageContext::Direct,
                },
            },
        };
        if shape.video_ids.is_empty()
            || shape.video_ids.len() > MAX_VIDEO_COUNT
            || shape
                .video_ids
                .windows(2)
                .any(|pair| pair.first() == pair.get(1))
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        Ok(shape)
    }
}

fn context_from_scope(
    scope: LegacyFolderAssignmentScopeV1,
    organization_id: &str,
) -> AtomicResult<StorageContext> {
    match scope {
        LegacyFolderAssignmentScopeV1::OrganizationLibrary {
            organization_id: scoped,
        } if scoped.to_string() == organization_id => Ok(StorageContext::Organization),
        LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. } => {
            Err(LegacyFolderAssignmentAtomicErrorV1::CrossTenant)
        }
        LegacyFolderAssignmentScopeV1::Space { space_id } => {
            Ok(StorageContext::Space(space_id.to_string()))
        }
    }
}

impl D1LegacyFolderAssignmentAtomicPortV1<'_> {
    async fn authority(&self, actor_id: &str, organization_id: &str) -> AtomicResult<AuthorityRow> {
        let mut rows = self
            .rows::<AuthorityRow>(
                AUTHORITY_SNAPSHOT_SQL,
                vec![js(actor_id), js(organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderAssignmentAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }

    async fn operation(
        &self,
        organization_id: &str,
        actor_id: &str,
        action: &str,
        key_digest: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(organization_id),
                    js(actor_id),
                    js(action),
                    js(key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
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
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderAssignmentAtomicErrorV1::TargetMissing)?;
        if row.id != folder_id
            || !nonnegative(&[row.revision, row.tree_revision])
            || (row.space_id.is_some()
                && !nonnegative(&[row.space_revision, row.space_authority_version]))
            || !valid_space_membership(&row.actor_space_role, row.actor_space_membership_revision)
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        parse_folder_id(&row.id)?;
        if let Some(space_id) = &row.space_id {
            parse_space_id(space_id)?;
        } else if row.space_revision != -1
            || row.space_authority_version != -1
            || !row.actor_space_role.is_empty()
            || row.actor_space_membership_revision != -1
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        if let Some(parent_id) = &row.parent_id {
            parse_folder_id(parent_id)?;
        }
        Ok(row)
    }

    async fn space_context(
        &self,
        space_id: &str,
        organization_id: &str,
        actor_id: &str,
    ) -> AtomicResult<SpaceContextRow> {
        let mut rows = self
            .rows::<SpaceContextRow>(
                SPACE_CONTEXT_SNAPSHOT_SQL,
                vec![js(space_id), js(organization_id), js(actor_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyFolderAssignmentAtomicErrorV1::TargetMissing)?;
        if row.id != space_id
            || !nonnegative(&[row.revision, row.authority_version])
            || !valid_space_membership(&row.actor_space_role, row.actor_space_membership_revision)
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        parse_space_id(&row.id)?;
        Ok(row)
    }

    async fn videos(
        &self,
        shape: &CommandShape,
        organization_id: &str,
    ) -> AtomicResult<Vec<VideoSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<VideoRow>(VIDEO_SNAPSHOT_SQL, vec![js(&ids_json), js(organization_id)])
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        rows.into_iter()
            .zip(&shape.video_ids)
            .map(|(row, requested)| {
                if row.requested_id != *requested {
                    return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
                }
                let (Some(id), Some(owner_id), Some(revision)) =
                    (row.id, row.owner_id, row.revision)
                else {
                    return Err(LegacyFolderAssignmentAtomicErrorV1::TargetMissing);
                };
                if id != *requested || revision < 0 {
                    return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
                }
                Ok(VideoSnapshot {
                    id,
                    owner_id,
                    folder_id: row.folder_id,
                    revision,
                })
            })
            .collect()
    }

    async fn space_assignments(
        &self,
        shape: &CommandShape,
        space_id: &str,
    ) -> AtomicResult<Vec<SpaceAssignmentSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<SpaceAssignmentRow>(
                SPACE_ASSIGNMENT_SNAPSHOT_SQL,
                vec![js(&ids_json), js(space_id)],
            )
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        rows.into_iter()
            .zip(&shape.video_ids)
            .map(|(row, requested)| {
                if row.requested_id != *requested {
                    return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
                }
                match (row.video_id, row.revision) {
                    (None, None) => Ok(SpaceAssignmentSnapshot {
                        id: requested.clone(),
                        present: false,
                        folder_id: None,
                        revision: -1,
                    }),
                    (Some(video_id), Some(revision)) if video_id == *requested && revision >= 0 => {
                        Ok(SpaceAssignmentSnapshot {
                            id: video_id,
                            present: true,
                            folder_id: row.folder_id,
                            revision,
                        })
                    }
                    _ => Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
                }
            })
            .collect()
    }

    async fn shared_assignments(
        &self,
        shape: &CommandShape,
        organization_id: &str,
        desired_folder_id: Option<&str>,
    ) -> AtomicResult<Vec<SharedAssignmentSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<SharedAssignmentSnapshot>(
                SHARED_ASSIGNMENT_SNAPSHOT_SQL,
                vec![
                    js(&ids_json),
                    js(organization_id),
                    js_opt(desired_folder_id),
                ],
            )
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        for (row, requested) in rows.iter().zip(&shape.video_ids) {
            if row.requested_id != *requested
                || !(0..=1).contains(&row.active_count)
                || (row.active_count == 0
                    && (!row.active_id.is_empty()
                        || row.active_folder_id.is_some()
                        || row.active_revision != -1))
                || (row.active_count == 1 && (row.active_id.is_empty() || row.active_revision < 0))
                || (row.dormant_id.is_empty() != (row.dormant_revision == -1))
                || (!row.dormant_id.is_empty() && row.dormant_revision < 0)
            {
                return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
            }
        }
        Ok(rows)
    }
}

fn nonnegative(values: &[i64]) -> bool {
    values.iter().all(|value| *value >= 0)
}

fn valid_space_membership(role: &str, revision: i64) -> bool {
    match role {
        "" => revision == -1,
        "manager" | "contributor" | "viewer" => revision >= 0,
        _ => false,
    }
}

fn authorize(
    authority: &AuthorityRow,
    actor_id: &str,
    shape: &CommandShape,
    videos: &[VideoSnapshot],
    target_folder: Option<&FolderRow>,
    context_space: Option<&SpaceContextRow>,
) -> AtomicResult<()> {
    let owns_every_video = videos.iter().all(|video| video.owner_id == actor_id);
    if shape.action != Action::Move && !owns_every_video {
        return Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied);
    }
    match authority.membership_role.as_str() {
        "owner" | "admin" => Ok(()),
        "viewer" => Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied),
        "member" => {
            let context_allowed = match context_space {
                None => owns_every_video,
                Some(space) if space.actor_space_role == "manager" => true,
                Some(space) if space.actor_space_role == "contributor" => owns_every_video,
                Some(_) => false,
            };
            let context_allowed = context_allowed
                || (shape.action == Action::Move
                    && context_space.is_none()
                    && target_folder.is_some_and(|folder| {
                        folder.actor_space_role == "manager"
                            || (folder.space_id.is_none() && folder.created_by_user_id == actor_id)
                    }));
            let folder_allowed = match target_folder {
                None => true,
                Some(folder) if folder.actor_space_role == "manager" => true,
                Some(folder) if folder.actor_space_role == "contributor" => {
                    folder.created_by_user_id == actor_id
                }
                Some(folder) if folder.space_id.is_none() => folder.created_by_user_id == actor_id,
                Some(_) => false,
            };
            (context_allowed && folder_allowed)
                .then_some(())
                .ok_or(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        }
        _ => Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
    }
}

fn original_folder_id(
    shape: &CommandShape,
    videos: &[VideoSnapshot],
    space: &[SpaceAssignmentSnapshot],
    shared: &[SharedAssignmentSnapshot],
) -> Option<String> {
    if shape.action != Action::Move {
        return None;
    }
    match shape.context {
        StorageContext::Direct => videos.first().and_then(|video| video.folder_id.clone()),
        StorageContext::Space(_) => space
            .first()
            .filter(|assignment| assignment.present)
            .and_then(|assignment| assignment.folder_id.clone()),
        StorageContext::Organization => shared
            .first()
            .filter(|assignment| assignment.active_count == 1)
            .and_then(|assignment| assignment.active_folder_id.clone()),
    }
}

fn authorized_context(
    command: &LegacyFolderAssignmentCommandV1,
    target_folder: Option<&FolderRow>,
    original_folder: Option<&FolderRow>,
) -> AtomicResult<LegacyFolderAssignmentAuthorizedContextV1> {
    let move_command = matches!(command, LegacyFolderAssignmentCommandV1::Move { .. });
    LegacyFolderAssignmentAuthorizedContextV1::new(
        command,
        target_folder
            .and_then(|folder| folder.space_id.as_deref())
            .map(parse_space_id)
            .transpose()?,
        original_folder
            .map(|folder| parse_folder_id(&folder.id))
            .transpose()?,
        original_folder
            .and_then(|folder| folder.parent_id.as_deref())
            .map(parse_folder_id)
            .transpose()?,
        if move_command {
            target_folder
                .and_then(|folder| folder.parent_id.as_deref())
                .map(parse_folder_id)
                .transpose()?
        } else {
            None
        },
    )
}

fn receipt_bundle(
    command: &LegacyFolderAssignmentCommandV1,
    shape: &CommandShape,
    target_folder: Option<&FolderRow>,
    original_folder: Option<&FolderRow>,
) -> AtomicResult<(LegacyFolderAssignmentMutationReceiptV1, String, String)> {
    let context = authorized_context(command, target_folder, original_folder)?;
    let mut targets = Vec::new();
    if let Some(target) = target_folder {
        let folder_id = parse_folder_id(&target.id)?;
        targets.push(LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id });
        if let Some(space_id) = target.space_id.as_deref() {
            targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id: parse_space_id(space_id)?,
                folder_id,
            });
        }
    }
    if let Some(original) = original_folder {
        targets.push(LegacyFolderAssignmentInvalidationTargetV1::Folder {
            folder_id: parse_folder_id(&original.id)?,
        });
    }
    if shape.action == Action::Move
        && original_folder.map(|folder| folder.id.as_str())
            != target_folder.map(|folder| folder.id.as_str())
    {
        for parent_id in [
            original_folder.and_then(|folder| folder.parent_id.as_deref()),
            target_folder.and_then(|folder| folder.parent_id.as_deref()),
        ]
        .into_iter()
        .flatten()
        {
            targets.push(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: parse_folder_id(parent_id)?,
            });
        }
    }
    if let StorageContext::Space(space_id) = &shape.context {
        targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceRoot {
            space_id: parse_space_id(space_id)?,
        });
    }
    let effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets(targets)
        .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
    let affected_count = match shape.action {
        Action::Add | Action::Remove => Some(
            u16::try_from(shape.video_ids.len())
                .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?,
        ),
        Action::Move => None,
    };
    let receipt =
        LegacyFolderAssignmentMutationReceiptV1::new(command, affected_count, effects, context)?;
    let effect_wire = effect_wire(receipt.effects());
    let context_wire = ContextWire {
        target_folder_space_id: context
            .target_folder_space_id()
            .map(|value| value.to_string()),
        original_folder_id: context.original_folder_id().map(|value| value.to_string()),
        original_parent_id: context.original_parent_id().map(|value| value.to_string()),
        target_parent_id: context.target_parent_id().map(|value| value.to_string()),
    };
    let effect_json = serde_json::to_string(&effect_wire)
        .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
    let receipt_json = serde_json::to_string(&ReceiptWire {
        affected_count,
        effects: effect_wire,
        authorized_context: context_wire,
    })
    .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
    if effect_json.len() > 2_048 || receipt_json.len() > 8_192 {
        return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
    }
    Ok((receipt, receipt_json, effect_json))
}

fn effect_wire(effects: &LegacyFolderAssignmentEffectsV1) -> EffectWire {
    let targets = effects
        .targets()
        .iter()
        .map(|target| match target {
            LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id } => {
                EffectTargetWire::Folder {
                    folder_id: folder_id.to_string(),
                }
            }
            LegacyFolderAssignmentInvalidationTargetV1::SpaceRoot { space_id } => {
                EffectTargetWire::SpaceRoot {
                    space_id: space_id.to_string(),
                }
            }
            LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id,
                folder_id,
            } => EffectTargetWire::SpaceFolder {
                space_id: space_id.to_string(),
                folder_id: folder_id.to_string(),
            },
        })
        .collect();
    EffectWire {
        invalidates_caps: effects.invalidates_caps(),
        targets,
    }
}

fn decode_receipt(
    command: &LegacyFolderAssignmentCommandV1,
    source: &str,
) -> AtomicResult<LegacyFolderAssignmentMutationReceiptV1> {
    let wire: ReceiptWire =
        serde_json::from_str(source).map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
    if !wire.effects.invalidates_caps {
        return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
    }
    let targets = wire
        .effects
        .targets
        .into_iter()
        .map(|target| match target {
            EffectTargetWire::Folder { folder_id } => {
                Ok(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                    folder_id: parse_folder_id(&folder_id)?,
                })
            }
            EffectTargetWire::SpaceRoot { space_id } => {
                Ok(LegacyFolderAssignmentInvalidationTargetV1::SpaceRoot {
                    space_id: parse_space_id(&space_id)?,
                })
            }
            EffectTargetWire::SpaceFolder {
                space_id,
                folder_id,
            } => Ok(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id: parse_space_id(&space_id)?,
                folder_id: parse_folder_id(&folder_id)?,
            }),
        })
        .collect::<AtomicResult<Vec<_>>>()?;
    let effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets(targets)
        .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
    let context = LegacyFolderAssignmentAuthorizedContextV1::new(
        command,
        wire.authorized_context
            .target_folder_space_id
            .as_deref()
            .map(parse_space_id)
            .transpose()?,
        wire.authorized_context
            .original_folder_id
            .as_deref()
            .map(parse_folder_id)
            .transpose()?,
        wire.authorized_context
            .original_parent_id
            .as_deref()
            .map(parse_folder_id)
            .transpose()?,
        wire.authorized_context
            .target_parent_id
            .as_deref()
            .map(parse_folder_id)
            .transpose()?,
    )?;
    LegacyFolderAssignmentMutationReceiptV1::new(command, wire.affected_count, effects, context)
}

fn parse_folder_id(value: &str) -> AtomicResult<FolderId> {
    FolderId::parse(value).map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)
}

fn parse_space_id(value: &str) -> AtomicResult<SpaceId> {
    SpaceId::parse(value).map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)
}

impl D1LegacyFolderAssignmentAtomicPortV1<'_> {
    #[allow(clippy::too_many_arguments)]
    fn product_plan(
        &self,
        shape: &CommandShape,
        videos: &[VideoSnapshot],
        space_assignments: &[SpaceAssignmentSnapshot],
        shared_assignments: &[SharedAssignmentSnapshot],
        organization_id: &str,
        actor_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> AtomicResult<(String, String, Vec<D1PreparedStatement>)> {
        let scoped_pre = match shape.context {
            StorageContext::Direct => Vec::new(),
            StorageContext::Space(_) => space_assignments
                .iter()
                .map(|assignment| {
                    json!({
                        "id": assignment.id,
                        "present": i64::from(assignment.present),
                        "folder_id": assignment.folder_id,
                        "revision": assignment.revision,
                    })
                })
                .collect(),
            StorageContext::Organization => {
                let desired = desired_shared_lookup(shape);
                shared_assignments
                    .iter()
                    .map(|assignment| {
                        json!({
                            "id": assignment.requested_id,
                            "active_count": assignment.active_count,
                            "active_id": assignment.active_id,
                            "active_folder_id": assignment.active_folder_id,
                            "active_revision": assignment.active_revision,
                            "desired_folder_id": desired,
                            "dormant_id": assignment.dormant_id,
                            "dormant_revision": assignment.dormant_revision,
                        })
                    })
                    .collect()
            }
        };
        let scoped_pre_json = serde_json::to_string(&scoped_pre)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;

        let mut final_videos = videos.to_vec();
        let mut final_space = space_assignments.to_vec();
        let mut final_shared = shared_assignments.to_vec();
        let mut mutations = Vec::new();

        for video in &mut final_videos {
            let desired = match shape.action {
                Action::Add => video.folder_id.clone(),
                Action::Remove if video.folder_id.as_deref() == shape.folder_id.as_deref() => None,
                Action::Remove => video.folder_id.clone(),
                Action::Move if shape.context == StorageContext::Direct => shape.folder_id.clone(),
                Action::Move => video.folder_id.clone(),
            };
            if desired != video.folder_id {
                mutations.push(self.statement(
                    DIRECT_ASSIGNMENT_SET_SQL,
                    vec![
                        js(&video.id),
                        js_opt(desired.as_deref()),
                        js(operation_id),
                        number(now_ms),
                        js(organization_id),
                        number(video.revision),
                        js_opt(video.folder_id.as_deref()),
                    ],
                )?);
                video.folder_id = desired;
                video.revision += 1;
            }
        }

        if let StorageContext::Space(space_id) = &shape.context {
            for assignment in &mut final_space {
                let desired = match shape.action {
                    Action::Add => shape.folder_id.clone(),
                    Action::Remove
                        if assignment.present
                            && assignment.folder_id.as_deref() == shape.folder_id.as_deref() =>
                    {
                        None
                    }
                    Action::Remove => assignment.folder_id.clone(),
                    Action::Move => shape.folder_id.clone(),
                };
                if !assignment.present {
                    if shape.action == Action::Add {
                        mutations.push(self.statement(
                            SPACE_ASSIGNMENT_INSERT_SQL,
                            vec![
                                js(space_id),
                                js(&assignment.id),
                                js_opt(desired.as_deref()),
                                js(actor_id),
                                number(now_ms),
                                js(operation_id),
                            ],
                        )?);
                        assignment.present = true;
                        assignment.folder_id = desired;
                        assignment.revision = 0;
                    }
                } else if desired != assignment.folder_id {
                    mutations.push(self.statement(
                        SPACE_ASSIGNMENT_SET_SQL,
                        vec![
                            js(space_id),
                            js(&assignment.id),
                            js_opt(desired.as_deref()),
                            js(operation_id),
                            number(assignment.revision),
                            js_opt(assignment.folder_id.as_deref()),
                        ],
                    )?);
                    assignment.folder_id = desired;
                    assignment.revision += 1;
                }
            }
        }

        if shape.context == StorageContext::Organization {
            for assignment in &mut final_shared {
                let desired = desired_shared_final(shape, assignment);
                if assignment.active_count == 0 {
                    if shape.action != Action::Add {
                        continue;
                    }
                    if assignment.dormant_id.is_empty() {
                        let membership_id = Uuid::now_v7().to_string();
                        let Some(folder_id) = desired.as_deref() else {
                            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
                        };
                        mutations.push(self.statement(
                            SHARED_ASSIGNMENT_INSERT_SQL,
                            vec![
                                js(&membership_id),
                                js(&assignment.requested_id),
                                js(organization_id),
                                js(folder_id),
                                js(actor_id),
                                number(now_ms),
                                js(operation_id),
                            ],
                        )?);
                        assignment.active_count = 1;
                        assignment.active_id = membership_id;
                        assignment.active_folder_id = desired;
                        assignment.active_revision = 0;
                    } else {
                        mutations.push(self.statement(
                            SHARED_ASSIGNMENT_REACTIVATE_SQL,
                            vec![
                                js(&assignment.dormant_id),
                                js(actor_id),
                                number(now_ms),
                                js(operation_id),
                                js(organization_id),
                                number(assignment.dormant_revision),
                                js_opt(desired.as_deref()),
                            ],
                        )?);
                        assignment.active_count = 1;
                        assignment.active_id = assignment.dormant_id.clone();
                        assignment.active_folder_id = desired;
                        assignment.active_revision = assignment.dormant_revision + 1;
                    }
                } else if desired != assignment.active_folder_id {
                    if assignment.dormant_id.is_empty() {
                        mutations.push(self.statement(
                            SHARED_ASSIGNMENT_SET_SQL,
                            vec![
                                js(&assignment.active_id),
                                js_opt(desired.as_deref()),
                                js(operation_id),
                                js(organization_id),
                                number(assignment.active_revision),
                                js_opt(assignment.active_folder_id.as_deref()),
                            ],
                        )?);
                        assignment.active_folder_id = desired;
                        assignment.active_revision += 1;
                    } else {
                        mutations.push(self.statement(
                            SHARED_ASSIGNMENT_REVOKE_SQL,
                            vec![
                                js(&assignment.active_id),
                                number(now_ms),
                                js(operation_id),
                                js(organization_id),
                                number(assignment.active_revision),
                            ],
                        )?);
                        mutations.push(self.statement(
                            SHARED_ASSIGNMENT_REACTIVATE_SQL,
                            vec![
                                js(&assignment.dormant_id),
                                js(actor_id),
                                number(now_ms),
                                js(operation_id),
                                js(organization_id),
                                number(assignment.dormant_revision),
                                js_opt(desired.as_deref()),
                            ],
                        )?);
                        assignment.active_id = assignment.dormant_id.clone();
                        assignment.active_folder_id = desired;
                        assignment.active_revision = assignment.dormant_revision + 1;
                    }
                }
            }
        }

        let final_json = serde_json::to_string(
            &final_videos
                .iter()
                .enumerate()
                .map(|(index, video)| {
                    let space = final_space.get(index);
                    let shared = final_shared.get(index);
                    json!({
                        "id": video.id,
                        "video_folder_id": video.folder_id,
                        "video_revision": video.revision,
                        "scope_id": shape.context.space_id(),
                        "scope_present": space.map_or(0, |value| i64::from(value.present)),
                        "scope_folder_id": space.and_then(|value| value.folder_id.as_deref()),
                        "scope_revision": space.map_or(-1, |value| value.revision),
                        "active_count": shared.map_or(0, |value| value.active_count),
                        "active_id": shared.map_or("", |value| value.active_id.as_str()),
                        "active_folder_id": shared.and_then(|value| value.active_folder_id.as_deref()),
                        "active_revision": shared.map_or(-1, |value| value.active_revision),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
        Ok((scoped_pre_json, final_json, mutations))
    }
}

fn desired_shared_lookup(shape: &CommandShape) -> Option<&str> {
    match shape.action {
        Action::Add | Action::Move => shape.folder_id.as_deref(),
        Action::Remove => None,
    }
}

fn desired_shared_final(
    shape: &CommandShape,
    assignment: &SharedAssignmentSnapshot,
) -> Option<String> {
    match shape.action {
        Action::Add | Action::Move => shape.folder_id.clone(),
        Action::Remove if assignment.active_folder_id.as_deref() == shape.folder_id.as_deref() => {
            None
        }
        Action::Remove => assignment.active_folder_id.clone(),
    }
}

impl D1LegacyFolderAssignmentAtomicPortV1<'_> {
    fn organization_assertion(
        &self,
        operation_id: &str,
        organization_id: &str,
        authority: &AuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            ORGANIZATION_ASSERT_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                number(authority.organization_revision),
                number(authority.organization_authority_version),
            ],
        )
    }

    fn selection_assertion(
        &self,
        operation_id: &str,
        actor_id: &str,
        organization_id: &str,
        authority: &AuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            SELECTION_ASSERT_SQL,
            vec![
                js(operation_id),
                js(actor_id),
                js(organization_id),
                number(authority.selection_revision),
            ],
        )
    }

    fn membership_assertion(
        &self,
        operation_id: &str,
        actor_id: &str,
        organization_id: &str,
        authority: &AuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            MEMBERSHIP_ASSERT_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                js(actor_id),
                js(&authority.membership_role),
                number(authority.membership_revision),
                number(authority.membership_authority_version),
            ],
        )
    }

    fn grant_assertion(
        &self,
        operation_id: &str,
        fence: LegacyFolderAssignmentBrowserFenceV1,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_MUTATION_GRANT_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                number(now_ms),
            ],
        )
    }

    fn grant_delete(
        &self,
        fence: LegacyFolderAssignmentBrowserFenceV1,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_MUTATION_GRANT_DELETE_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
            ],
        )
    }

    fn change_assertion(
        &self,
        operation_id: &str,
        kind: &str,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_MUTATION_CHANGE_ASSERT_SQL,
            vec![js(operation_id), js(kind)],
        )
    }

    fn cleanup(&self, operation_id: &str) -> AtomicResult<D1PreparedStatement> {
        self.statement(ASSERTION_CLEANUP_SQL, vec![js(operation_id)])
    }

    async fn consume_grant_only(
        &self,
        fence: LegacyFolderAssignmentBrowserFenceV1,
    ) -> AtomicResult<()> {
        let assertion_id = Uuid::now_v7().to_string();
        let now_ms = current_time_ms()?;
        self.batch(vec![
            self.grant_assertion(&assertion_id, fence, now_ms)?,
            self.grant_delete(fence)?,
            self.change_assertion(&assertion_id, "grant_consumed")?,
            self.cleanup(&assertion_id)?,
        ])
        .await
    }

    async fn consume_replay(
        &self,
        operation_id: &str,
        actor_id: &str,
        organization_id: &str,
        fence: LegacyFolderAssignmentBrowserFenceV1,
    ) -> AtomicResult<()> {
        let authority = self.authority(actor_id, organization_id).await?;
        let now_ms = current_time_ms()?;
        self.batch(vec![
            self.organization_assertion(operation_id, organization_id, &authority)?,
            self.selection_assertion(operation_id, actor_id, organization_id, &authority)?,
            self.membership_assertion(operation_id, actor_id, organization_id, &authority)?,
            self.grant_assertion(operation_id, fence, now_ms)?,
            self.grant_delete(fence)?,
            self.change_assertion(operation_id, "grant_consumed")?,
            self.cleanup(operation_id)?,
        ])
        .await
    }

    #[allow(clippy::too_many_arguments)]
    fn product_precondition(
        &self,
        operation_id: &str,
        organization_id: &str,
        actor_id: &str,
        shape: &CommandShape,
        target: Option<&FolderRow>,
        context_space: Option<&SpaceContextRow>,
        videos_json: &str,
        scoped_json: &str,
        original: Option<&FolderRow>,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PRODUCT_PRECONDITION_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                js(actor_id),
                js(shape.context.label()),
                js(target.map_or("", |folder| folder.id.as_str())),
                number(target.map_or(-1, |folder| folder.revision)),
                number(target.map_or(-1, |folder| folder.tree_revision)),
                js_opt(target.and_then(|folder| folder.space_id.as_deref())),
                js(target.map_or("", |folder| folder.created_by_user_id.as_str())),
                number(target.map_or(-1, |folder| folder.space_revision)),
                number(target.map_or(-1, |folder| folder.space_authority_version)),
                js(target.map_or("", |folder| folder.actor_space_role.as_str())),
                number(target.map_or(-1, |folder| folder.actor_space_membership_revision)),
                js(context_space.map_or("", |space| space.id.as_str())),
                number(context_space.map_or(-1, |space| space.revision)),
                number(context_space.map_or(-1, |space| space.authority_version)),
                js(context_space.map_or("", |space| space.actor_space_role.as_str())),
                number(context_space.map_or(-1, |space| space.actor_space_membership_revision)),
                js(videos_json),
                js(scoped_json),
                js_opt(target.and_then(|folder| folder.parent_id.as_deref())),
                js(original.map_or("", |folder| folder.id.as_str())),
                number(original.map_or(-1, |folder| folder.revision)),
                number(original.map_or(-1, |folder| folder.tree_revision)),
                js_opt(original.and_then(|folder| folder.space_id.as_deref())),
                js_opt(original.and_then(|folder| folder.parent_id.as_deref())),
                number(original.map_or(-1, |folder| folder.space_revision)),
                number(original.map_or(-1, |folder| folder.space_authority_version)),
                js(shape.action.policy_name()),
            ],
        )
    }
}

impl D1LegacyFolderAssignmentAtomicPortV1<'_> {
    #[allow(clippy::too_many_arguments)]
    async fn execute_fresh(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        shape: &CommandShape,
        fence: LegacyFolderAssignmentBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
        key_digest: &str,
        request_digest: &str,
    ) -> AtomicResult<LegacyFolderAssignmentMutationReceiptV1> {
        let authority = self.authority(actor_id, organization_id).await?;
        let target_folder = match shape.folder_id.as_deref() {
            Some(folder_id) => Some(self.folder(folder_id, organization_id, actor_id).await?),
            None => None,
        };
        let context_space = match shape.context.space_id() {
            Some(space_id) => Some(
                self.space_context(space_id, organization_id, actor_id)
                    .await?,
            ),
            None => None,
        };
        if let (Some(target), Some(space)) = (&target_folder, &context_space)
            && target.space_id.as_deref() != Some(space.id.as_str())
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::CrossTenant);
        }
        let videos = self.videos(shape, organization_id).await?;
        authorize(
            &authority,
            actor_id,
            shape,
            &videos,
            target_folder.as_ref(),
            context_space.as_ref(),
        )?;
        let space_assignments = match shape.context.space_id() {
            Some(space_id) => self.space_assignments(shape, space_id).await?,
            None => Vec::new(),
        };
        let shared_assignments = if shape.context == StorageContext::Organization {
            self.shared_assignments(shape, organization_id, desired_shared_lookup(shape))
                .await?
        } else {
            Vec::new()
        };
        let original_id =
            original_folder_id(shape, &videos, &space_assignments, &shared_assignments);
        let original_folder = match original_id.as_deref() {
            None => None,
            Some(original_id)
                if target_folder.as_ref().map(|folder| folder.id.as_str()) == Some(original_id) =>
            {
                target_folder.clone()
            }
            Some(original_id) => Some(self.folder(original_id, organization_id, actor_id).await?),
        };
        for (parent_id, child_space) in [
            target_folder.as_ref().and_then(|folder| {
                folder
                    .parent_id
                    .as_deref()
                    .map(|parent_id| (parent_id, folder.space_id.as_deref()))
            }),
            original_folder.as_ref().and_then(|folder| {
                folder
                    .parent_id
                    .as_deref()
                    .map(|parent_id| (parent_id, folder.space_id.as_deref()))
            }),
        ]
        .into_iter()
        .flatten()
        {
            let parent = self.folder(parent_id, organization_id, actor_id).await?;
            if parent.space_id.as_deref() != child_space {
                return Err(LegacyFolderAssignmentAtomicErrorV1::CrossTenant);
            }
        }
        if let (Some(original), Some(space)) = (&original_folder, &context_space)
            && original.space_id.as_deref() != Some(space.id.as_str())
        {
            return Err(LegacyFolderAssignmentAtomicErrorV1::CrossTenant);
        }

        let operation_id = Uuid::now_v7().to_string();
        let now_ms = current_time_ms()?;
        let videos_json = serde_json::to_string(&videos)
            .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
        let (scoped_json, final_json, mutations) = self.product_plan(
            shape,
            &videos,
            &space_assignments,
            &shared_assignments,
            organization_id,
            actor_id,
            &operation_id,
            now_ms,
        )?;
        let (receipt, receipt_json, effect_json) = receipt_bundle(
            command,
            shape,
            target_folder.as_ref(),
            original_folder.as_ref(),
        )?;
        let action = shape.action.journal_name();
        let principal_digest = digest_fields(
            b"frame.legacy-folder-assignment.principal.v1\0",
            &[actor_id],
        );
        let subject_digest = digest_fields(
            b"frame.legacy-folder-assignment.subject.v1\0",
            &[organization_id, action, request_digest],
        );
        let audit_id = Uuid::now_v7().to_string();

        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(actor_id),
                    js(action),
                    js(key_digest),
                    js(request_digest),
                    number(now_ms),
                ],
            )?,
            self.organization_assertion(&operation_id, organization_id, &authority)?,
            self.selection_assertion(&operation_id, actor_id, organization_id, &authority)?,
            self.membership_assertion(&operation_id, actor_id, organization_id, &authority)?,
            self.grant_assertion(&operation_id, fence, now_ms)?,
            self.product_precondition(
                &operation_id,
                organization_id,
                actor_id,
                shape,
                target_folder.as_ref(),
                context_space.as_ref(),
                &videos_json,
                &scoped_json,
                original_folder.as_ref(),
            )?,
        ];
        statements.extend(mutations);
        statements.extend([
            self.statement(
                PRODUCT_POSTCONDITION_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(&final_json),
                    js(shape.context.label()),
                ],
            )?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(actor_id),
                    js(action),
                    js(&effect_json),
                    number(now_ms),
                ],
            )?,
            self.change_assertion(&operation_id, "action_effect")?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![js(&operation_id), js(&receipt_json), number(now_ms)],
            )?,
            self.change_assertion(&operation_id, "operation_complete")?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    js(&audit_id),
                    js(&operation_id),
                    js(organization_id),
                    js(&principal_digest),
                    js(AUDIT_ACTION),
                    js(&subject_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                RECEIPT_POSTCONDITION_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(actor_id),
                    js(action),
                    js(&receipt_json),
                    js(&effect_json),
                    js(AUDIT_ACTION),
                ],
            )?,
            self.grant_delete(fence)?,
            self.change_assertion(&operation_id, "grant_consumed")?,
            self.cleanup(&operation_id)?,
        ]);
        self.batch(statements).await?;
        Ok(receipt)
    }

    async fn replay_existing(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        operation: &OperationRow,
        fence: LegacyFolderAssignmentBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<LegacyFolderAssignmentAtomicOutcomeV1> {
        if Uuid::parse_str(&operation.operation_id).is_err() {
            let _ = self.consume_grant_only(fence).await;
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        match (
            operation.state.as_str(),
            operation.response_json.as_deref(),
            operation.effect_json.as_deref(),
            operation.audit_count,
        ) {
            ("complete", Some(response_json), Some(effect_json), 1) => {
                let receipt = match decode_receipt(command, response_json) {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        let _ = self.consume_grant_only(fence).await;
                        return Err(error);
                    }
                };
                let canonical_effect = serde_json::to_string(&effect_wire(receipt.effects()))
                    .map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Corrupt)?;
                if canonical_effect != effect_json {
                    let _ = self.consume_grant_only(fence).await;
                    return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
                }
                self.consume_replay(&operation.operation_id, actor_id, organization_id, fence)
                    .await?;
                Ok(LegacyFolderAssignmentAtomicOutcomeV1::Replay(receipt))
            }
            ("claimed", None, None, 0) => {
                self.consume_grant_only(fence).await?;
                Err(LegacyFolderAssignmentAtomicErrorV1::InFlight)
            }
            _ => {
                let _ = self.consume_grant_only(fence).await;
                Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reconcile(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        fence: LegacyFolderAssignmentBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
        action: &str,
        key_digest: &str,
        request_digest: &str,
        original_error: LegacyFolderAssignmentAtomicErrorV1,
    ) -> AtomicResult<LegacyFolderAssignmentAtomicOutcomeV1> {
        match self
            .operation(organization_id, actor_id, action, key_digest)
            .await
        {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                self.replay_existing(command, &operation, fence, actor_id, organization_id)
                    .await
            }
            Ok(Some(_)) => {
                self.consume_grant_only(fence).await?;
                Err(LegacyFolderAssignmentAtomicErrorV1::Conflict)
            }
            Ok(None) => {
                self.consume_grant_only(fence).await?;
                Err(original_error)
            }
            Err(_) => {
                let _ = self.consume_grant_only(fence).await;
                Err(LegacyFolderAssignmentAtomicErrorV1::Unavailable)
            }
        }
    }
}

#[async_trait]
impl LegacyFolderAssignmentAtomicPortV1 for D1LegacyFolderAssignmentAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        browser_fence: &LegacyFolderAssignmentBrowserFenceV1,
    ) -> AtomicResult<LegacyFolderAssignmentAtomicOutcomeV1> {
        let actor_id = command.fence().authority().actor_id().to_string();
        let organization_id = command
            .fence()
            .authority()
            .active_organization_id()
            .to_string();
        let fence = *browser_fence;
        if fence.actor_id().to_string() != actor_id {
            let _ = self.consume_grant_only(fence).await;
            return Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied);
        }
        let shape = match CommandShape::from_command(command) {
            Ok(shape) => shape,
            Err(error) => {
                let _ = self.consume_grant_only(fence).await;
                return Err(error);
            }
        };
        let action = shape.action.journal_name();
        let key_digest = idempotency_key_digest(
            &organization_id,
            &actor_id,
            action,
            command.fence().idempotency_key().expose(),
        );
        let request_digest = lower_hex(command.fence().request_fingerprint());

        match self
            .operation(&organization_id, &actor_id, action, &key_digest)
            .await
        {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                return self
                    .replay_existing(command, &operation, fence, &actor_id, &organization_id)
                    .await;
            }
            Ok(Some(_)) => {
                self.consume_grant_only(fence).await?;
                return Err(LegacyFolderAssignmentAtomicErrorV1::Conflict);
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self.consume_grant_only(fence).await;
                return Err(error);
            }
        }

        match self
            .execute_fresh(
                command,
                &shape,
                fence,
                &actor_id,
                &organization_id,
                &key_digest,
                &request_digest,
            )
            .await
        {
            Ok(receipt) => Ok(LegacyFolderAssignmentAtomicOutcomeV1::Applied(receipt)),
            Err(error) => {
                self.reconcile(
                    command,
                    fence,
                    &actor_id,
                    &organization_id,
                    action,
                    &key_digest,
                    &request_digest,
                    error,
                )
                .await
            }
        }
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

fn current_time_ms() -> AtomicResult<i64> {
    let now = js_sys::Date::now();
    if !now.is_finite() || !(0.0..=9_007_199_254_740_991.0).contains(&now) {
        return Err(LegacyFolderAssignmentAtomicErrorV1::Unavailable);
    }
    Ok(now as i64)
}

fn idempotency_key_digest(
    organization_id: &str,
    actor_id: &str,
    action: &str,
    raw_key: &str,
) -> String {
    digest_fields(
        b"frame.legacy-folder-assignment.idempotency-key.v1\0",
        &[organization_id, actor_id, action, raw_key],
    )
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
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(role: &str) -> AuthorityRow {
        AuthorityRow {
            selection_revision: 1,
            organization_revision: 2,
            organization_authority_version: 3,
            membership_role: role.into(),
            membership_revision: 4,
            membership_authority_version: 5,
        }
    }

    fn video(owner_id: &str) -> VideoSnapshot {
        VideoSnapshot {
            id: Uuid::now_v7().to_string(),
            owner_id: owner_id.into(),
            folder_id: None,
            revision: 0,
        }
    }

    fn folder(role: &str, creator: &str) -> FolderRow {
        FolderRow {
            id: Uuid::now_v7().to_string(),
            space_id: Some(Uuid::now_v7().to_string()),
            parent_id: None,
            created_by_user_id: creator.into(),
            revision: 0,
            tree_revision: 0,
            space_revision: 0,
            space_authority_version: 0,
            actor_space_role: role.into(),
            actor_space_membership_revision: 0,
        }
    }

    fn personal_folder(creator: &str) -> FolderRow {
        FolderRow {
            id: Uuid::now_v7().to_string(),
            space_id: None,
            parent_id: None,
            created_by_user_id: creator.into(),
            revision: 0,
            tree_revision: 0,
            space_revision: -1,
            space_authority_version: -1,
            actor_space_role: String::new(),
            actor_space_membership_revision: -1,
        }
    }

    fn space(role: &str) -> SpaceContextRow {
        SpaceContextRow {
            id: Uuid::now_v7().to_string(),
            revision: 0,
            authority_version: 0,
            actor_space_role: role.into(),
            actor_space_membership_revision: 0,
        }
    }

    fn shape(action: Action, context: StorageContext) -> CommandShape {
        CommandShape {
            action,
            folder_id: None,
            video_ids: vec![Uuid::now_v7().to_string()],
            context,
        }
    }

    #[test]
    fn idempotency_key_is_tenant_actor_action_scoped_and_never_retained() {
        let first = idempotency_key_digest("tenant-a", "actor-a", "add", "raw-secret-key");
        assert_eq!(first.len(), 64);
        assert!(!first.contains("raw-secret-key"));
        assert_eq!(
            first,
            idempotency_key_digest("tenant-a", "actor-a", "add", "raw-secret-key")
        );
        assert_ne!(
            first,
            idempotency_key_digest("tenant-b", "actor-a", "add", "raw-secret-key")
        );
        assert_ne!(
            first,
            idempotency_key_digest("tenant-a", "actor-a", "move", "raw-secret-key")
        );
    }

    #[test]
    fn every_list_query_is_json_bounded_and_tenant_joined() {
        for sql in [
            VIDEO_SNAPSHOT_SQL,
            SPACE_ASSIGNMENT_SNAPSHOT_SQL,
            SHARED_ASSIGNMENT_SNAPSHOT_SQL,
        ] {
            assert!(sql.contains("json_each(?1)"));
            assert!(sql.contains("LIMIT 501"));
            assert!(!sql.contains("legacy_api_execution_operations_v1"));
        }
        assert!(VIDEO_SNAPSHOT_SQL.contains("v.organization_id = ?2"));
        assert!(SHARED_ASSIGNMENT_SNAPSHOT_SQL.contains("organization_id = ?2"));
        assert!(SPACE_ASSIGNMENT_SNAPSHOT_SQL.contains("sv.space_id = ?2"));
    }

    #[test]
    fn authorization_matrix_fails_closed_without_silent_video_filtering() {
        let actor = "actor";
        let owned = video(actor);
        let foreign = video("different-owner");
        let add = shape(Action::Add, StorageContext::Organization);
        let move_direct = shape(Action::Move, StorageContext::Direct);
        let move_space = shape(Action::Move, StorageContext::Space(space("manager").id));
        assert_eq!(
            authorize(
                &authority("owner"),
                actor,
                &add,
                std::slice::from_ref(&foreign),
                None,
                None,
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert!(
            authorize(
                &authority("member"),
                actor,
                &add,
                std::slice::from_ref(&owned),
                Some(&personal_folder(actor)),
                None,
            )
            .is_ok()
        );
        assert!(
            authorize(
                &authority("member"),
                actor,
                &move_direct,
                std::slice::from_ref(&foreign),
                Some(&personal_folder(actor)),
                None,
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &authority("member"),
                actor,
                &move_direct,
                std::slice::from_ref(&foreign),
                Some(&personal_folder("another-member")),
                None,
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert!(
            authorize(
                &authority("owner"),
                actor,
                &move_direct,
                std::slice::from_ref(&foreign),
                None,
                None,
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &authority("member"),
                actor,
                &add,
                std::slice::from_ref(&foreign),
                Some(&folder("manager", "someone")),
                Some(&space("manager")),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert_eq!(
            authorize(
                &authority("viewer"),
                actor,
                &move_direct,
                std::slice::from_ref(&owned),
                None,
                None,
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert_eq!(
            authorize(
                &authority("member"),
                actor,
                &move_direct,
                std::slice::from_ref(&foreign),
                None,
                None,
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert!(
            authorize(
                &authority("member"),
                actor,
                &move_space,
                std::slice::from_ref(&foreign),
                Some(&folder("manager", "someone")),
                Some(&space("manager")),
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &authority("member"),
                actor,
                &move_space,
                &[foreign],
                Some(&folder("contributor", actor)),
                Some(&space("contributor")),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::AccessDenied)
        );
        assert!(
            authorize(
                &authority("member"),
                actor,
                &move_space,
                &[owned],
                Some(&folder("contributor", actor)),
                Some(&space("contributor")),
            )
            .is_ok()
        );
    }

    #[test]
    fn operation_surface_uses_business_audit_and_authenticated_receipts_only() {
        for sql in [
            OPERATION_CLAIM_SQL,
            EFFECT_INSERT_SQL,
            OPERATION_COMPLETE_SQL,
            AUDIT_INSERT_SQL,
            RECEIPT_POSTCONDITION_SQL,
        ] {
            assert!(!sql.contains("legacy_api_execution_operations_v1"));
            assert!(!sql.contains("legacy_api_execution_effects_v1"));
        }
        assert!(OPERATION_CLAIM_SQL.contains("authenticated_web_action_operations_v1"));
        assert!(EFFECT_INSERT_SQL.contains("authenticated_web_action_effects_v1"));
        assert!(AUDIT_INSERT_SQL.contains("business_audit_events_v1"));
        assert!(RECEIPT_POSTCONDITION_SQL.contains("effect.value_json = ?6"));
    }
}
