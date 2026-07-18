//! Atomic D1 adapter for Cap's four library-root placement actions.
//!
//! The source handlers silently filtered videos and split authority, placement,
//! cache effects, and success reporting across statements.  This adapter uses
//! only normalized `shared_videos` / `space_videos` membership, reasserts every
//! bounded snapshot plus the one-use browser grant in one D1 batch, and stores
//! the exact action result and invalidation effect for deterministic replay.

use async_trait::async_trait;
use frame_application::{
    LegacyFolderAssignmentScopeV1, LegacyLibraryPlacementActionV1,
    LegacyLibraryPlacementAtomicErrorV1, LegacyLibraryPlacementAtomicOutcomeV1,
    LegacyLibraryPlacementAtomicPortV1, LegacyLibraryPlacementBrowserFenceV1,
    LegacyLibraryPlacementCommandV1, LegacyLibraryPlacementEffectsV1,
    LegacyLibraryPlacementMutationReceiptV1, LegacyLibraryPlacementMutationResultV1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, send::IntoSendFuture};

const AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_library_placement/authority_snapshot.sql");
const SPACE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_library_placement/space_snapshot.sql");
const VIDEO_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_library_placement/video_snapshot.sql");
const SHARED_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_snapshot.sql");
const SPACE_MEMBERSHIP_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_library_placement/space_membership_snapshot.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_library_placement/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_library_placement/operation_claim.sql");
const ORGANIZATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/organization_assert.sql");
const SELECTION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/selection_assert.sql");
const MEMBERSHIP_ASSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/membership_assert.sql");
const PLACEMENT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/placement_authority_assert.sql");
const PRODUCT_PRECONDITION_SQL: &str =
    include_str!("../queries/legacy_library_placement/product_precondition.sql");
const PRODUCT_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_library_placement/product_postcondition.sql");
const SHARED_ROOT_INSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_root_insert.sql");
const SHARED_ROOT_SET_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_root_set.sql");
const SHARED_ROOT_REACTIVATE_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_root_reactivate.sql");
const SHARED_REVOKE_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_revoke.sql");
const SHARED_DELETE_SQL: &str =
    include_str!("../queries/legacy_library_placement/shared_delete.sql");
const SPACE_ROOT_INSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/space_root_insert.sql");
const SPACE_ROOT_SET_SQL: &str =
    include_str!("../queries/legacy_library_placement/space_root_set.sql");
const SPACE_DELETE_SQL: &str = include_str!("../queries/legacy_library_placement/space_delete.sql");
const DIRECT_FOLDER_CLEAR_SQL: &str =
    include_str!("../queries/legacy_library_placement/direct_folder_clear.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_library_placement/effect_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_library_placement/operation_complete.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_library_placement/audit_insert.sql");
const RECEIPT_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_library_placement/receipt_postcondition.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_library_placement/assertion_cleanup.sql");
const BROWSER_MUTATION_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_assert.sql");
const BROWSER_MUTATION_GRANT_DELETE_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_delete_by_proof.sql");
const BROWSER_MUTATION_CHANGE_ASSERT_SQL: &str =
    include_str!("../queries/auth/browser_mutation_change_assert.sql");

const MAX_VIDEO_COUNT: usize = 500;
const AUDIT_ACTION: &str = "legacy.library_placement";

type AtomicResult<T> = Result<T, LegacyLibraryPlacementAtomicErrorV1>;

pub(crate) struct D1LegacyLibraryPlacementAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyLibraryPlacementAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Unavailable)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Unavailable);
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
        if ![
            self.selection_revision,
            self.organization_revision,
            self.organization_authority_version,
            self.membership_revision,
            self.membership_authority_version,
        ]
        .into_iter()
        .all(|value| value >= 0)
            || !matches!(
                self.membership_role.as_str(),
                "owner" | "admin" | "member" | "viewer"
            )
        {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SpaceRow {
    id: String,
    revision: i64,
    authority_version: i64,
    actor_space_role: String,
    actor_space_membership_revision: i64,
}

impl SpaceRow {
    fn validate(&self, expected_id: &str) -> AtomicResult<()> {
        let valid_membership = match self.actor_space_role.as_str() {
            "" => self.actor_space_membership_revision == -1,
            "manager" | "contributor" | "viewer" => self.actor_space_membership_revision >= 0,
            _ => false,
        };
        if self.id != expected_id
            || Uuid::parse_str(&self.id).is_err()
            || self.revision < 0
            || self.authority_version < 0
            || !valid_membership
        {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
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

#[derive(Debug, Deserialize)]
struct VideoRow {
    requested_id: String,
    id: Option<String>,
    owner_id: Option<String>,
    folder_id: Option<String>,
    revision: Option<i64>,
    folder_in_tenant: i64,
}

#[derive(Debug, Clone, Serialize)]
struct VideoSnapshot {
    id: String,
    present: i64,
    owner_id: Option<String>,
    folder_id: Option<String>,
    revision: i64,
    folder_in_tenant: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedSnapshot {
    #[serde(rename = "id", alias = "requested_id")]
    requested_id: String,
    active_count: i64,
    active_id: String,
    active_folder_id: Option<String>,
    active_sharing_mode: String,
    active_revision: i64,
    dormant_root_id: String,
    dormant_root_revision: i64,
}

#[derive(Debug, Deserialize)]
struct SpaceMembershipRow {
    requested_id: String,
    video_id: Option<String>,
    folder_id: Option<String>,
    revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct SpaceMembershipSnapshot {
    id: String,
    present: i64,
    folder_id: Option<String>,
    revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    AddOrganization,
    RemoveOrganization,
    AddScope,
    RemoveScope,
}

impl Action {
    const fn journal_name(self) -> &'static str {
        match self {
            Self::AddOrganization => "legacy_library_add_organization_v1",
            Self::RemoveOrganization => "legacy_library_remove_organization_v1",
            Self::AddScope => "legacy_library_add_scope_v1",
            Self::RemoveScope => "legacy_library_remove_scope_v1",
        }
    }

    const fn policy_name(self) -> &'static str {
        match self {
            Self::AddOrganization => "add_organization",
            Self::RemoveOrganization => "remove_organization",
            Self::AddScope => "add_scope",
            Self::RemoveScope => "remove_scope",
        }
    }

    const fn requires_actor_owned_videos(self) -> bool {
        !matches!(self, Self::RemoveOrganization)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScopeContext {
    Organization,
    Space(String),
}

impl ScopeContext {
    const fn label(&self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Space(_) => "space",
        }
    }

    fn space_id(&self) -> Option<&str> {
        match self {
            Self::Organization => None,
            Self::Space(space_id) => Some(space_id),
        }
    }
}

#[derive(Debug, Clone)]
struct CommandShape {
    action: Action,
    scope: ScopeContext,
    video_ids: Vec<String>,
}

impl CommandShape {
    fn from_command(command: &LegacyLibraryPlacementCommandV1) -> AtomicResult<Self> {
        let organization_id = command
            .fence()
            .authority()
            .active_organization_id()
            .to_string();
        let action = match command.action() {
            LegacyLibraryPlacementActionV1::AddToOrganization => Action::AddOrganization,
            LegacyLibraryPlacementActionV1::RemoveFromOrganization => Action::RemoveOrganization,
            LegacyLibraryPlacementActionV1::AddToSpace => Action::AddScope,
            LegacyLibraryPlacementActionV1::RemoveFromSpace => Action::RemoveScope,
        };
        let scope = match command.scope() {
            LegacyFolderAssignmentScopeV1::OrganizationLibrary {
                organization_id: scoped,
            } if scoped.to_string() == organization_id => ScopeContext::Organization,
            LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. } => {
                return Err(LegacyLibraryPlacementAtomicErrorV1::CrossTenant);
            }
            LegacyFolderAssignmentScopeV1::Space { space_id } => {
                ScopeContext::Space(space_id.to_string())
            }
        };
        if matches!(action, Action::AddOrganization | Action::RemoveOrganization)
            && scope != ScopeContext::Organization
        {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        let video_ids = command
            .video_ids()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if video_ids.is_empty()
            || video_ids.len() > MAX_VIDEO_COUNT
            || video_ids.windows(2).any(|pair| pair.first() >= pair.get(1))
            || video_ids
                .iter()
                .any(|value| Uuid::parse_str(value).is_err())
        {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            action,
            scope,
            video_ids,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
enum ScopeWire {
    Organization { organization_id: String },
    Space { space_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectWire {
    scope: ScopeWire,
    invalidates_scope_root: bool,
    invalidates_caps: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
enum ResultWire {
    OrganizationAdded { total_updated: u16 },
    OrganizationRemoved { existing_shared: u16 },
    ScopeAdded { valid_video_count: u16 },
    ScopeRemoved { valid_video_count: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptWire {
    result: ResultWire,
}

#[derive(Debug, Clone, Serialize)]
struct FinalRow {
    id: String,
    video_present: i64,
    video_folder_id: Option<String>,
    video_revision: i64,
    active_count: i64,
    active_id: String,
    active_folder_id: Option<String>,
    active_sharing_mode: String,
    active_revision: i64,
    scope_present: i64,
    scope_folder_id: Option<String>,
    scope_revision: i64,
}

impl D1LegacyLibraryPlacementAtomicPortV1<'_> {
    async fn authority(&self, actor_id: &str, organization_id: &str) -> AtomicResult<AuthorityRow> {
        let mut rows = self
            .rows::<AuthorityRow>(
                AUTHORITY_SNAPSHOT_SQL,
                vec![js(actor_id), js(organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyLibraryPlacementAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }

    async fn space(
        &self,
        space_id: &str,
        organization_id: &str,
        actor_id: &str,
    ) -> AtomicResult<SpaceRow> {
        let mut rows = self
            .rows::<SpaceRow>(
                SPACE_SNAPSHOT_SQL,
                vec![js(space_id), js(organization_id), js(actor_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyLibraryPlacementAtomicErrorV1::TargetMissing)?;
        row.validate(space_id)?;
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
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn videos(
        &self,
        shape: &CommandShape,
        organization_id: &str,
    ) -> AtomicResult<Vec<VideoSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<VideoRow>(VIDEO_SNAPSHOT_SQL, vec![js(&ids_json), js(organization_id)])
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        rows.into_iter()
            .zip(&shape.video_ids)
            .map(|(row, requested)| {
                if row.requested_id != *requested || !(0..=1).contains(&row.folder_in_tenant) {
                    return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                }
                match (row.id, row.owner_id, row.revision) {
                    (None, None, None) if row.folder_id.is_none() && row.folder_in_tenant == 0 => {
                        Ok(VideoSnapshot {
                            id: requested.clone(),
                            present: 0,
                            owner_id: None,
                            folder_id: None,
                            revision: -1,
                            folder_in_tenant: 0,
                        })
                    }
                    (Some(id), Some(owner_id), Some(revision))
                        if id == *requested && revision >= 0 =>
                    {
                        Ok(VideoSnapshot {
                            id,
                            present: 1,
                            owner_id: Some(owner_id),
                            folder_id: row.folder_id,
                            revision,
                            folder_in_tenant: row.folder_in_tenant,
                        })
                    }
                    _ => Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt),
                }
            })
            .collect()
    }

    async fn shared(
        &self,
        shape: &CommandShape,
        organization_id: &str,
    ) -> AtomicResult<Vec<SharedSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<SharedSnapshot>(
                SHARED_SNAPSHOT_SQL,
                vec![js(&ids_json), js(organization_id)],
            )
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        for (row, requested) in rows.iter().zip(&shape.video_ids) {
            let active_valid = match row.active_count {
                0 => {
                    row.active_id.is_empty()
                        && row.active_folder_id.is_none()
                        && row.active_sharing_mode.is_empty()
                        && row.active_revision == -1
                }
                1 => {
                    Uuid::parse_str(&row.active_id).is_ok()
                        && matches!(
                            row.active_sharing_mode.as_str(),
                            "organization" | "space" | "public_link"
                        )
                        && row.active_revision >= 0
                }
                _ => false,
            };
            let dormant_valid = if row.dormant_root_id.is_empty() {
                row.dormant_root_revision == -1
            } else {
                Uuid::parse_str(&row.dormant_root_id).is_ok() && row.dormant_root_revision >= 0
            };
            if row.requested_id != *requested || !active_valid || !dormant_valid {
                return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
            }
        }
        Ok(rows)
    }

    async fn space_memberships(
        &self,
        shape: &CommandShape,
        space_id: &str,
    ) -> AtomicResult<Vec<SpaceMembershipSnapshot>> {
        let ids_json = serde_json::to_string(&shape.video_ids)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let rows = self
            .rows::<SpaceMembershipRow>(
                SPACE_MEMBERSHIP_SNAPSHOT_SQL,
                vec![js(&ids_json), js(space_id)],
            )
            .await?;
        if rows.len() != shape.video_ids.len() {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        rows.into_iter()
            .zip(&shape.video_ids)
            .map(|(row, requested)| {
                if row.requested_id != *requested {
                    return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                }
                match (row.video_id, row.revision) {
                    (None, None) if row.folder_id.is_none() => Ok(SpaceMembershipSnapshot {
                        id: requested.clone(),
                        present: 0,
                        folder_id: None,
                        revision: -1,
                    }),
                    (Some(video_id), Some(revision)) if video_id == *requested && revision >= 0 => {
                        Ok(SpaceMembershipSnapshot {
                            id: video_id,
                            present: 1,
                            folder_id: row.folder_id,
                            revision,
                        })
                    }
                    _ => Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt),
                }
            })
            .collect()
    }
}

fn authorize_manager(authority: &AuthorityRow, space: Option<&SpaceRow>) -> AtomicResult<()> {
    match authority.membership_role.as_str() {
        "owner" | "admin" => Ok(()),
        "member" if space.is_some_and(|value| value.actor_space_role == "manager") => Ok(()),
        "member" | "viewer" => Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied),
        _ => Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt),
    }
}

fn authorize(
    authority: &AuthorityRow,
    space: Option<&SpaceRow>,
    shape: &CommandShape,
    videos: &[VideoSnapshot],
    actor_id: &str,
) -> AtomicResult<()> {
    authorize_manager(authority, space)?;
    if shape.action.requires_actor_owned_videos() {
        if videos.iter().any(|video| video.present != 1) {
            return Err(LegacyLibraryPlacementAtomicErrorV1::TargetMissing);
        }
        if videos
            .iter()
            .any(|video| video.owner_id.as_deref() != Some(actor_id))
        {
            return Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied);
        }
    }
    Ok(())
}

fn result_wire(result: LegacyLibraryPlacementMutationResultV1) -> ResultWire {
    match result {
        LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated } => {
            ResultWire::OrganizationAdded { total_updated }
        }
        LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared } => {
            ResultWire::OrganizationRemoved { existing_shared }
        }
        LegacyLibraryPlacementMutationResultV1::ScopeAdded { valid_video_count } => {
            ResultWire::ScopeAdded { valid_video_count }
        }
        LegacyLibraryPlacementMutationResultV1::ScopeRemoved { valid_video_count } => {
            ResultWire::ScopeRemoved { valid_video_count }
        }
    }
}

fn mutation_result(wire: ResultWire) -> LegacyLibraryPlacementMutationResultV1 {
    match wire {
        ResultWire::OrganizationAdded { total_updated } => {
            LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated }
        }
        ResultWire::OrganizationRemoved { existing_shared } => {
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared }
        }
        ResultWire::ScopeAdded { valid_video_count } => {
            LegacyLibraryPlacementMutationResultV1::ScopeAdded { valid_video_count }
        }
        ResultWire::ScopeRemoved { valid_video_count } => {
            LegacyLibraryPlacementMutationResultV1::ScopeRemoved { valid_video_count }
        }
    }
}

fn effect_wire(effects: &LegacyLibraryPlacementEffectsV1) -> EffectWire {
    let scope = match effects.scope() {
        LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id } => {
            ScopeWire::Organization {
                organization_id: organization_id.to_string(),
            }
        }
        LegacyFolderAssignmentScopeV1::Space { space_id } => ScopeWire::Space {
            space_id: space_id.to_string(),
        },
    };
    EffectWire {
        scope,
        invalidates_scope_root: effects.invalidates_scope_root(),
        invalidates_caps: effects.invalidates_caps(),
    }
}

fn receipt_bundle(
    command: &LegacyLibraryPlacementCommandV1,
    result: LegacyLibraryPlacementMutationResultV1,
) -> AtomicResult<(LegacyLibraryPlacementMutationReceiptV1, String, String)> {
    let receipt = LegacyLibraryPlacementMutationReceiptV1::new(command, result)?;
    let receipt_json = serde_json::to_string(&ReceiptWire {
        result: result_wire(result),
    })
    .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
    let effect_json = serde_json::to_string(&effect_wire(receipt.effects()))
        .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
    if receipt_json.len() > 8_192 || effect_json.len() > 2_048 {
        return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
    }
    Ok((receipt, receipt_json, effect_json))
}

fn decode_receipt(
    command: &LegacyLibraryPlacementCommandV1,
    source: &str,
) -> AtomicResult<LegacyLibraryPlacementMutationReceiptV1> {
    let wire: ReceiptWire =
        serde_json::from_str(source).map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
    LegacyLibraryPlacementMutationReceiptV1::new(command, mutation_result(wire.result))
}

impl D1LegacyLibraryPlacementAtomicPortV1<'_> {
    #[allow(clippy::too_many_arguments)]
    fn product_plan(
        &self,
        shape: &CommandShape,
        videos: &[VideoSnapshot],
        shared: &[SharedSnapshot],
        space_memberships: &[SpaceMembershipSnapshot],
        organization_id: &str,
        actor_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> AtomicResult<(Vec<D1PreparedStatement>, String, u16)> {
        let mut final_videos = videos.to_vec();
        let mut final_shared = shared.to_vec();
        let mut final_space = space_memberships.to_vec();
        let mut mutations = Vec::new();
        let mut existing_shared = 0_u16;

        match &shape.scope {
            ScopeContext::Organization => {
                if final_shared.len() != shape.video_ids.len() || !final_space.is_empty() {
                    return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                }
                for (index, membership) in final_shared.iter_mut().enumerate() {
                    match shape.action {
                        Action::AddOrganization | Action::AddScope => {
                            if membership.active_count == 0 {
                                if membership.dormant_root_id.is_empty() {
                                    let id = Uuid::now_v7().to_string();
                                    mutations.push(self.statement(
                                        SHARED_ROOT_INSERT_SQL,
                                        vec![
                                            js(&id),
                                            js(&membership.requested_id),
                                            js(organization_id),
                                            js(actor_id),
                                            number(now_ms),
                                            js(operation_id),
                                        ],
                                    )?);
                                    membership.active_id = id;
                                    membership.active_revision = 0;
                                } else {
                                    mutations.push(self.statement(
                                        SHARED_ROOT_REACTIVATE_SQL,
                                        vec![
                                            js(&membership.dormant_root_id),
                                            js(actor_id),
                                            number(now_ms),
                                            js(operation_id),
                                            js(organization_id),
                                            number(membership.dormant_root_revision),
                                        ],
                                    )?);
                                    membership.active_id = membership.dormant_root_id.clone();
                                    membership.active_revision =
                                        membership.dormant_root_revision + 1;
                                }
                            } else if membership.active_folder_id.is_some()
                                || membership.active_sharing_mode != "organization"
                            {
                                if membership.dormant_root_id.is_empty() {
                                    mutations.push(self.statement(
                                        SHARED_ROOT_SET_SQL,
                                        vec![
                                            js(&membership.active_id),
                                            js(operation_id),
                                            js(organization_id),
                                            number(membership.active_revision),
                                            js_opt(membership.active_folder_id.as_deref()),
                                            js(&membership.active_sharing_mode),
                                        ],
                                    )?);
                                    membership.active_revision += 1;
                                } else {
                                    mutations.push(self.statement(
                                        SHARED_REVOKE_SQL,
                                        vec![
                                            js(&membership.active_id),
                                            number(now_ms),
                                            js(operation_id),
                                            js(organization_id),
                                            number(membership.active_revision),
                                        ],
                                    )?);
                                    mutations.push(self.statement(
                                        SHARED_ROOT_REACTIVATE_SQL,
                                        vec![
                                            js(&membership.dormant_root_id),
                                            js(actor_id),
                                            number(now_ms),
                                            js(operation_id),
                                            js(organization_id),
                                            number(membership.dormant_root_revision),
                                        ],
                                    )?);
                                    membership.active_id = membership.dormant_root_id.clone();
                                    membership.active_revision =
                                        membership.dormant_root_revision + 1;
                                }
                            }
                            membership.active_count = 1;
                            membership.active_folder_id = None;
                            membership.active_sharing_mode = "organization".into();
                        }
                        Action::RemoveOrganization | Action::RemoveScope => {
                            if membership.active_count == 1 {
                                mutations.push(self.statement(
                                    SHARED_DELETE_SQL,
                                    vec![
                                        js(&membership.active_id),
                                        js(organization_id),
                                        number(membership.active_revision),
                                    ],
                                )?);
                                if shape.action == Action::RemoveOrganization {
                                    existing_shared = existing_shared
                                        .checked_add(1)
                                        .ok_or(LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
                                    let video = final_videos
                                        .get_mut(index)
                                        .ok_or(LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
                                    if video.present == 1
                                        && video.folder_in_tenant == 1
                                        && video.folder_id.is_some()
                                    {
                                        let old_folder = video
                                            .folder_id
                                            .clone()
                                            .ok_or(LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
                                        mutations.push(self.statement(
                                            DIRECT_FOLDER_CLEAR_SQL,
                                            vec![
                                                js(&video.id),
                                                js(organization_id),
                                                js(operation_id),
                                                number(now_ms),
                                                number(video.revision),
                                                js(&old_folder),
                                            ],
                                        )?);
                                        video.folder_id = None;
                                        video.revision += 1;
                                        video.folder_in_tenant = 0;
                                    }
                                }
                                membership.active_count = 0;
                                membership.active_id.clear();
                                membership.active_folder_id = None;
                                membership.active_sharing_mode.clear();
                                membership.active_revision = -1;
                            }
                        }
                    }
                }
            }
            ScopeContext::Space(space_id) => {
                if !final_shared.is_empty()
                    || final_space.len() != shape.video_ids.len()
                    || !matches!(shape.action, Action::AddScope | Action::RemoveScope)
                {
                    return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                }
                for membership in &mut final_space {
                    match shape.action {
                        Action::AddScope if membership.present == 0 => {
                            mutations.push(self.statement(
                                SPACE_ROOT_INSERT_SQL,
                                vec![
                                    js(space_id),
                                    js(&membership.id),
                                    js(actor_id),
                                    number(now_ms),
                                    js(operation_id),
                                ],
                            )?);
                            membership.present = 1;
                            membership.folder_id = None;
                            membership.revision = 0;
                        }
                        Action::AddScope if membership.folder_id.is_some() => {
                            mutations.push(self.statement(
                                SPACE_ROOT_SET_SQL,
                                vec![
                                    js(space_id),
                                    js(&membership.id),
                                    js(operation_id),
                                    number(membership.revision),
                                    js_opt(membership.folder_id.as_deref()),
                                ],
                            )?);
                            membership.folder_id = None;
                            membership.revision += 1;
                        }
                        Action::RemoveScope if membership.present == 1 => {
                            mutations.push(self.statement(
                                SPACE_DELETE_SQL,
                                vec![
                                    js(space_id),
                                    js(&membership.id),
                                    number(membership.revision),
                                ],
                            )?);
                            membership.present = 0;
                            membership.folder_id = None;
                            membership.revision = -1;
                        }
                        Action::AddScope | Action::RemoveScope => {}
                        Action::AddOrganization | Action::RemoveOrganization => {
                            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                        }
                    }
                }
            }
        }

        let final_rows = final_videos
            .iter()
            .enumerate()
            .map(|(index, video)| {
                let shared = final_shared.get(index);
                let space = final_space.get(index);
                FinalRow {
                    id: video.id.clone(),
                    video_present: video.present,
                    video_folder_id: video.folder_id.clone(),
                    video_revision: video.revision,
                    active_count: shared.map_or(0, |value| value.active_count),
                    active_id: shared.map_or_else(String::new, |value| value.active_id.clone()),
                    active_folder_id: shared.and_then(|value| value.active_folder_id.clone()),
                    active_sharing_mode: shared
                        .map_or_else(String::new, |value| value.active_sharing_mode.clone()),
                    active_revision: shared.map_or(-1, |value| value.active_revision),
                    scope_present: space.map_or(0, |value| value.present),
                    scope_folder_id: space.and_then(|value| value.folder_id.clone()),
                    scope_revision: space.map_or(-1, |value| value.revision),
                }
            })
            .collect::<Vec<_>>();
        let final_json = serde_json::to_string(&final_rows)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        Ok((mutations, final_json, existing_shared))
    }
}

impl D1LegacyLibraryPlacementAtomicPortV1<'_> {
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

    fn placement_authority_assertion(
        &self,
        operation_id: &str,
        organization_id: &str,
        actor_id: &str,
        authority: &AuthorityRow,
        space: Option<&SpaceRow>,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PLACEMENT_AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                js(actor_id),
                js(&authority.membership_role),
                js(space.map_or("", |value| value.id.as_str())),
                number(space.map_or(-1, |value| value.revision)),
                number(space.map_or(-1, |value| value.authority_version)),
                number(space.map_or(-1, |value| value.actor_space_membership_revision)),
                js(space.map_or("", |value| value.actor_space_role.as_str())),
            ],
        )
    }

    fn grant_assertion(
        &self,
        operation_id: &str,
        fence: LegacyLibraryPlacementBrowserFenceV1,
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
        fence: LegacyLibraryPlacementBrowserFenceV1,
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
        fence: LegacyLibraryPlacementBrowserFenceV1,
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
        shape: &CommandShape,
        fence: LegacyLibraryPlacementBrowserFenceV1,
    ) -> AtomicResult<()> {
        let authority = match self.authority(actor_id, organization_id).await {
            Ok(authority) => authority,
            Err(error) => {
                let _ = self.consume_grant_only(fence).await;
                return Err(error);
            }
        };
        let space = match shape.scope.space_id() {
            Some(space_id) => match self.space(space_id, organization_id, actor_id).await {
                Ok(space) => Some(space),
                Err(error) => {
                    let _ = self.consume_grant_only(fence).await;
                    return Err(error);
                }
            },
            None => None,
        };
        if let Err(error) = authorize_manager(&authority, space.as_ref()) {
            let _ = self.consume_grant_only(fence).await;
            return Err(error);
        }
        let now_ms = current_time_ms()?;
        self.batch(vec![
            self.organization_assertion(operation_id, organization_id, &authority)?,
            self.selection_assertion(operation_id, actor_id, organization_id, &authority)?,
            self.membership_assertion(operation_id, actor_id, organization_id, &authority)?,
            self.placement_authority_assertion(
                operation_id,
                organization_id,
                actor_id,
                &authority,
                space.as_ref(),
            )?,
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
        authority: &AuthorityRow,
        space: Option<&SpaceRow>,
        videos_json: &str,
        scoped_json: &str,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PRODUCT_PRECONDITION_SQL,
            vec![
                js(operation_id),
                js(organization_id),
                js(actor_id),
                js(shape.action.policy_name()),
                js(shape.scope.label()),
                js(space.map_or("", |value| value.id.as_str())),
                number(space.map_or(-1, |value| value.revision)),
                number(space.map_or(-1, |value| value.authority_version)),
                js(space.map_or("", |value| value.actor_space_role.as_str())),
                number(space.map_or(-1, |value| value.actor_space_membership_revision)),
                js(videos_json),
                js(scoped_json),
                js(&authority.membership_role),
            ],
        )
    }
}

impl D1LegacyLibraryPlacementAtomicPortV1<'_> {
    #[allow(clippy::too_many_arguments)]
    async fn execute_fresh(
        &self,
        command: &LegacyLibraryPlacementCommandV1,
        shape: &CommandShape,
        fence: LegacyLibraryPlacementBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
        key_digest: &str,
        request_digest: &str,
    ) -> AtomicResult<LegacyLibraryPlacementMutationReceiptV1> {
        let authority = self.authority(actor_id, organization_id).await?;
        let space = match shape.scope.space_id() {
            Some(space_id) => Some(self.space(space_id, organization_id, actor_id).await?),
            None => None,
        };
        let videos = self.videos(shape, organization_id).await?;
        authorize(&authority, space.as_ref(), shape, &videos, actor_id)?;
        let (shared, space_memberships) = match &shape.scope {
            ScopeContext::Organization => (self.shared(shape, organization_id).await?, Vec::new()),
            ScopeContext::Space(space_id) => {
                (Vec::new(), self.space_memberships(shape, space_id).await?)
            }
        };
        let operation_id = Uuid::now_v7().to_string();
        let now_ms = current_time_ms()?;
        let videos_json = serde_json::to_string(&videos)
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let scoped_json = match &shape.scope {
            ScopeContext::Organization => serde_json::to_string(&shared),
            ScopeContext::Space(_) => serde_json::to_string(&space_memberships),
        }
        .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let (mutations, final_json, existing_shared) = self.product_plan(
            shape,
            &videos,
            &shared,
            &space_memberships,
            organization_id,
            actor_id,
            &operation_id,
            now_ms,
        )?;
        let requested = u16::try_from(shape.video_ids.len())
            .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
        let result = match shape.action {
            Action::AddOrganization => LegacyLibraryPlacementMutationResultV1::OrganizationAdded {
                total_updated: requested,
            },
            Action::RemoveOrganization => {
                LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared }
            }
            Action::AddScope => LegacyLibraryPlacementMutationResultV1::ScopeAdded {
                valid_video_count: requested,
            },
            Action::RemoveScope => LegacyLibraryPlacementMutationResultV1::ScopeRemoved {
                valid_video_count: requested,
            },
        };
        let (receipt, receipt_json, effect_json) = receipt_bundle(command, result)?;
        let action = shape.action.journal_name();
        let principal_digest = digest_fields(
            b"frame.legacy-library-placement.principal.v1\0",
            &[actor_id],
        );
        let subject_digest = digest_fields(
            b"frame.legacy-library-placement.subject.v1\0",
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
                &authority,
                space.as_ref(),
                &videos_json,
                &scoped_json,
            )?,
        ];
        statements.extend(mutations);
        statements.extend([
            self.statement(
                PRODUCT_POSTCONDITION_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(shape.scope.label()),
                    js(shape.scope.space_id().unwrap_or("")),
                    js(&final_json),
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
        command: &LegacyLibraryPlacementCommandV1,
        shape: &CommandShape,
        operation: &OperationRow,
        fence: LegacyLibraryPlacementBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<LegacyLibraryPlacementAtomicOutcomeV1> {
        if Uuid::parse_str(&operation.operation_id).is_err() {
            let _ = self.consume_grant_only(fence).await;
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
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
                    .map_err(|_| LegacyLibraryPlacementAtomicErrorV1::Corrupt)?;
                if canonical_effect != effect_json {
                    let _ = self.consume_grant_only(fence).await;
                    return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
                }
                self.consume_replay(
                    &operation.operation_id,
                    actor_id,
                    organization_id,
                    shape,
                    fence,
                )
                .await?;
                Ok(LegacyLibraryPlacementAtomicOutcomeV1::Replay(receipt))
            }
            ("claimed", None, None, 0) => {
                self.consume_grant_only(fence).await?;
                Err(LegacyLibraryPlacementAtomicErrorV1::InFlight)
            }
            _ => {
                let _ = self.consume_grant_only(fence).await;
                Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reconcile(
        &self,
        command: &LegacyLibraryPlacementCommandV1,
        shape: &CommandShape,
        fence: LegacyLibraryPlacementBrowserFenceV1,
        actor_id: &str,
        organization_id: &str,
        action: &str,
        key_digest: &str,
        request_digest: &str,
        original_error: LegacyLibraryPlacementAtomicErrorV1,
    ) -> AtomicResult<LegacyLibraryPlacementAtomicOutcomeV1> {
        match self
            .operation(organization_id, actor_id, action, key_digest)
            .await
        {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                self.replay_existing(command, shape, &operation, fence, actor_id, organization_id)
                    .await
            }
            Ok(Some(_)) => {
                self.consume_grant_only(fence).await?;
                Err(LegacyLibraryPlacementAtomicErrorV1::Conflict)
            }
            Ok(None) => {
                self.consume_grant_only(fence).await?;
                Err(original_error)
            }
            Err(_) => {
                let _ = self.consume_grant_only(fence).await;
                Err(LegacyLibraryPlacementAtomicErrorV1::Unavailable)
            }
        }
    }
}

#[async_trait]
impl LegacyLibraryPlacementAtomicPortV1 for D1LegacyLibraryPlacementAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyLibraryPlacementCommandV1,
        browser_fence: &LegacyLibraryPlacementBrowserFenceV1,
    ) -> AtomicResult<LegacyLibraryPlacementAtomicOutcomeV1> {
        let actor_id = command.fence().authority().actor_id().to_string();
        let organization_id = command
            .fence()
            .authority()
            .active_organization_id()
            .to_string();
        let fence = *browser_fence;
        if fence.actor_id().to_string() != actor_id {
            let _ = self.consume_grant_only(fence).await;
            return Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied);
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
                    .replay_existing(
                        command,
                        &shape,
                        &operation,
                        fence,
                        &actor_id,
                        &organization_id,
                    )
                    .await;
            }
            Ok(Some(_)) => {
                self.consume_grant_only(fence).await?;
                return Err(LegacyLibraryPlacementAtomicErrorV1::Conflict);
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
            Ok(receipt) => Ok(LegacyLibraryPlacementAtomicOutcomeV1::Applied(receipt)),
            Err(error) => {
                self.reconcile(
                    command,
                    &shape,
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
        return Err(LegacyLibraryPlacementAtomicErrorV1::Unavailable);
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
        b"frame.legacy-library-placement.idempotency-key.v1\0",
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
    use frame_application::{
        LegacyFolderAssignmentCredentialV1, LegacyLibraryPlacementAdapterV1,
        LegacyLibraryPlacementInputV1, LegacyLibraryPlacementRequestV1,
    };
    use frame_domain::{LegacyCapNanoId, OrganizationId, UserId};

    use super::*;

    const ORGANIZATION: &str = "0123456789abcde";
    const VIDEO: &str = "1123456789abcde";

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

    fn space(role: &str) -> SpaceRow {
        SpaceRow {
            id: Uuid::now_v7().to_string(),
            revision: 1,
            authority_version: 2,
            actor_space_role: role.into(),
            actor_space_membership_revision: 3,
        }
    }

    fn video(owner_id: Option<&str>) -> VideoSnapshot {
        VideoSnapshot {
            id: Uuid::now_v7().to_string(),
            present: i64::from(owner_id.is_some()),
            owner_id: owner_id.map(String::from),
            folder_id: None,
            revision: if owner_id.is_some() { 0 } else { -1 },
            folder_in_tenant: 0,
        }
    }

    fn command() -> LegacyLibraryPlacementCommandV1 {
        let actor = UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor");
        let organization = LegacyCapNanoId::parse(ORGANIZATION)
            .expect("organization")
            .mapped_uuid()
            .to_string();
        LegacyLibraryPlacementAdapterV1::add_videos_to_organization()
            .prepare(&LegacyLibraryPlacementRequestV1 {
                credential: Some(LegacyFolderAssignmentCredentialV1::Session),
                actor_id: Some(actor),
                active_organization_id: Some(
                    OrganizationId::parse(&organization).expect("organization"),
                ),
                idempotency_key: Some("library-placement-test-1".into()),
                input: LegacyLibraryPlacementInputV1::AddToOrganization {
                    legacy_organization_id: ORGANIZATION.into(),
                    legacy_video_ids: vec![VIDEO.into()],
                },
            })
            .expect("command")
    }

    #[test]
    fn idempotency_digest_is_tenant_actor_and_action_scoped() {
        let first = idempotency_key_digest("tenant-a", "actor-a", "add", "secret-key");
        assert_eq!(first.len(), 64);
        assert!(!first.contains("secret-key"));
        assert_eq!(
            first,
            idempotency_key_digest("tenant-a", "actor-a", "add", "secret-key")
        );
        assert_ne!(
            first,
            idempotency_key_digest("tenant-b", "actor-a", "add", "secret-key")
        );
        assert_ne!(
            first,
            idempotency_key_digest("tenant-a", "actor-a", "remove", "secret-key")
        );
    }

    #[test]
    fn every_list_snapshot_is_bounded_and_tenant_joined() {
        for sql in [
            VIDEO_SNAPSHOT_SQL,
            SHARED_SNAPSHOT_SQL,
            SPACE_MEMBERSHIP_SNAPSHOT_SQL,
        ] {
            assert!(sql.contains("json_each(?1)"));
            assert!(sql.contains("LIMIT 501"));
            assert!(!sql.contains("legacy_api_execution_operations_v1"));
        }
        assert!(VIDEO_SNAPSHOT_SQL.contains("v.organization_id = ?2"));
        assert!(SHARED_SNAPSHOT_SQL.contains("video.organization_id = ?2"));
        assert!(SPACE_MEMBERSHIP_SNAPSHOT_SQL.contains("membership.space_id = ?2"));
    }

    #[test]
    fn manager_and_ownership_asymmetry_is_fail_closed() {
        let actor = "actor";
        let owned = video(Some(actor));
        let foreign = video(Some("foreign"));
        let missing = video(None);
        let add_organization = CommandShape {
            action: Action::AddOrganization,
            scope: ScopeContext::Organization,
            video_ids: vec![owned.id.clone()],
        };
        let remove_organization = CommandShape {
            action: Action::RemoveOrganization,
            scope: ScopeContext::Organization,
            video_ids: vec![foreign.id.clone()],
        };
        assert_eq!(
            authorize(
                &authority("owner"),
                None,
                &add_organization,
                std::slice::from_ref(&foreign),
                actor,
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied)
        );
        assert_eq!(
            authorize(
                &authority("owner"),
                None,
                &add_organization,
                &[missing],
                actor,
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::TargetMissing)
        );
        assert!(
            authorize(
                &authority("owner"),
                None,
                &remove_organization,
                std::slice::from_ref(&foreign),
                actor,
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &authority("member"),
                None,
                &remove_organization,
                &[foreign],
                actor,
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied)
        );
        assert!(
            authorize(
                &authority("member"),
                Some(&space("manager")),
                &CommandShape {
                    action: Action::AddScope,
                    scope: ScopeContext::Space(Uuid::now_v7().to_string()),
                    video_ids: vec![owned.id.clone()],
                },
                &[owned],
                actor,
            )
            .is_ok()
        );
        assert_eq!(
            authorize_manager(&authority("member"), Some(&space("contributor"))),
            Err(LegacyLibraryPlacementAtomicErrorV1::AccessDenied)
        );
    }

    #[test]
    fn receipt_round_trip_preserves_exact_effects_and_rejects_wrong_count() {
        let command = command();
        let result = LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated: 1 };
        let (receipt, response_json, effect_json) =
            receipt_bundle(&command, result).expect("receipt");
        let decoded = decode_receipt(&command, &response_json).expect("decoded");
        assert_eq!(decoded, receipt);
        assert_eq!(
            serde_json::to_string(&effect_wire(decoded.effects())).expect("effect"),
            effect_json
        );
        assert_eq!(
            receipt_bundle(
                &command,
                LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated: 0 },
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn persistence_surface_is_business_only_and_mutations_are_scope_exact() {
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
        assert!(AUDIT_INSERT_SQL.contains("business_audit_events_v1"));
        assert!(SHARED_DELETE_SQL.contains("organization_id = ?2"));
        assert!(SPACE_DELETE_SQL.contains("space_id = ?1"));
        assert!(DIRECT_FOLDER_CLEAR_SQL.contains("folder.organization_id = ?2"));
        assert!(PRODUCT_PRECONDITION_SQL.contains("?4 = 'remove_organization'"));
        assert!(PRODUCT_PRECONDITION_SQL.contains("?9 = 'manager'"));
    }
}
