//! Atomic D1 adapter for Cap's six retained membership mutations.
//!
//! Every attempt consumes one browser grant. Successful mutations additionally
//! reassert active-tenant authority, persist an exact typed receipt, derive the
//! full affected authority set in SQLite, bump its compatibility generation,
//! revoke outstanding mutation grants, and commit all evidence in one batch.

use std::collections::BTreeMap;

use async_trait::async_trait;
use frame_application::{
    LegacyMembershipActorAuthorityV1, LegacyMembershipAtomicErrorV1,
    LegacyMembershipAtomicOutcomeV1, LegacyMembershipAtomicPortV1,
    LegacyMembershipAuthorityPostconditionV1, LegacyMembershipBrowserFenceV1,
    LegacyMembershipCommandV1, LegacyMembershipDiscoveredContextV1,
    LegacyMembershipMutationPostconditionV1, LegacyMembershipMutationReceiptV1,
    LegacyMembershipMutationResultV1, LegacySpaceMemberIdV1, LegacySpaceMemberRemovalTargetV1,
    LegacySpaceMemberTargetV1, LegacySpaceMemberUserAliasV1, MAX_LEGACY_DISCOVERED_SPACE_MEMBERS,
    MAX_LEGACY_MEMBERSHIP_TARGETS,
};
use frame_domain::{
    LegacyCapNanoId, OrganizationId, OrganizationInviteId, SessionId, SessionMutationGrantId,
    SpaceId, SpaceRole, TimestampMillis, UserId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_membership_actions/clock_now.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_membership_actions/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_membership_actions/operation_claim.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_membership_actions/operation_complete.sql");
const INVITE_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/invite_authority_snapshot.sql");
const SPACE_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/space_authority_snapshot.sql");
const INVITE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/invite_authority_assert.sql");
const SPACE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/space_authority_assert.sql");
const TENANT_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/tenant_authority_snapshot.sql");
const TENANT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/tenant_authority_assert.sql");
const BROWSER_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/browser_grant_assert.sql");
const BROWSER_GRANT_DELETE_RETURNING_SQL: &str =
    include_str!("../queries/legacy_membership_actions/browser_grant_delete_returning.sql");
const CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/changes_assert.sql");
const FINAL_MEMBERS_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/final_members_insert.sql");
const FINAL_CREATOR_UPSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/final_creator_upsert.sql");
const PREVIOUS_ALIASES_COMPLETE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/previous_aliases_complete_assert.sql");
const MEMBER_ALIAS_INSERT_ADDED_SQL: &str =
    include_str!("../queries/legacy_membership_actions/member_alias_insert_added.sql");
const MEMBER_ALIAS_INSERT_ALL_SQL: &str =
    include_str!("../queries/legacy_membership_actions/member_alias_insert_all.sql");
const MEMBER_ALIAS_REMOVE_PREVIOUS_SQL: &str =
    include_str!("../queries/legacy_membership_actions/member_alias_remove_previous.sql");
const MEMBER_ALIAS_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/member_alias_postcondition_assert.sql");
const MEMBER_ALIAS_ADDED_POSTCONDITION_ASSERT_SQL: &str = include_str!(
    "../queries/legacy_membership_actions/member_alias_added_postcondition_assert.sql"
);
const ALIASES_PREVIOUS_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/aliases_previous_changes_assert.sql");
const ALIASES_FINAL_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/aliases_final_changes_assert.sql");
const ALIASES_ADDED_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/aliases_added_changes_assert.sql");
const TARGET_GRAPH_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/target_graph_assert.sql");
const CREATOR_GRAPH_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/creator_graph_assert.sql");
const INVITE_TARGET_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/invite_target_assert.sql");
const ADD_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/add_absent_assert.sql");
const INVITE_DELETE_SQL: &str =
    include_str!("../queries/legacy_membership_actions/invite_delete.sql");
const INVITE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/invite_postcondition_assert.sql");
const ADD_INSERT_SQL: &str = include_str!("../queries/legacy_membership_actions/add_insert.sql");
const ADD_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/add_postcondition_assert.sql");
const BULK_ADD_DUPLICATE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/bulk_add_duplicate_assert.sql");
const BULK_ADD_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/bulk_add_insert.sql");
const BULK_ADD_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/bulk_add_postcondition_assert.sql");
const BULK_ADDED_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/bulk_added_changes_assert.sql");
const EFFECT_INSERT_BULK_ADD_SQL: &str =
    include_str!("../queries/legacy_membership_actions/effect_insert_bulk_add.sql");
const AUTHORITY_SUBJECT_INSERT_ADDED_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_subject_insert_added.sql");
const RECEIPT_INSERT_BULK_ADD_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_bulk_add.sql");
const MEMBER_ALIAS_TARGETS_SQL: &str =
    include_str!("../queries/legacy_membership_actions/member_alias_targets.sql");
const REMOVAL_TARGETS_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_targets_insert.sql");
const REMOVAL_TARGETS_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_targets_assert.sql");
const REMOVED_TARGET_GRAPH_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removed_target_graph_assert.sql");
const REMOVAL_NO_MATCH_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_no_match_assert.sql");
const REMOVAL_CREATOR_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_creator_assert.sql");
const REMOVAL_DELETE_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_delete.sql");
const REMOVAL_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_changes_assert.sql");
const REMOVAL_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/removal_postcondition_assert.sql");
const AUTHORITY_SUBJECT_INSERT_REMOVED_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_subject_insert_removed.sql");
const RECEIPT_INSERT_BATCH_REMOVE_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_batch_remove.sql");
const RECEIPT_INSERT_REMOVE_MEMBER_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_remove_member.sql");
const PREVIOUS_SNAPSHOT_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/previous_snapshot_insert.sql");
const PREVIOUS_BOUND_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/previous_bound_assert.sql");
const SET_DELETE_SQL: &str = include_str!("../queries/legacy_membership_actions/set_delete.sql");
const PREVIOUS_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/previous_changes_assert.sql");
const SET_INSERT_SQL: &str = include_str!("../queries/legacy_membership_actions/set_insert.sql");
const FINAL_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/final_changes_assert.sql");
const SET_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/set_postcondition_assert.sql");
const OUT_OF_SCOPE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/out_of_scope_assert.sql");
const AUTHORITY_SUBJECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_subject_insert.sql");
const AUTHORITY_GENERATION_UPSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_generation_upsert.sql");
const AUTHORITY_GENERATION_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_generation_changes_assert.sql");
const AUTHORITY_GENERATION_POSTCONDITION_ASSERT_SQL: &str = include_str!(
    "../queries/legacy_membership_actions/authority_generation_postcondition_assert.sql"
);
const REVOKED_GRANT_SNAPSHOT_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/revoked_grant_snapshot_insert.sql");
const REVOKE_GRANTS_SQL: &str =
    include_str!("../queries/legacy_membership_actions/revoke_grants.sql");
const REVOKED_GRANT_CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/revoked_grant_changes_assert.sql");
const REVOKED_GRANT_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/revoked_grant_postcondition_assert.sql");
const RECEIPT_INSERT_INVITE_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_invite.sql");
const RECEIPT_INSERT_ADD_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_add.sql");
const RECEIPT_INSERT_SET_SQL: &str =
    include_str!("../queries/legacy_membership_actions/receipt_insert_set.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/effect_insert.sql");
const AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/audit_insert.sql");
const PROOF_INSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/proof_insert.sql");
const DURABLE_RECEIPT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_membership_actions/durable_receipt_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_membership_actions/assertion_cleanup.sql");
const PREVIOUS_MEMBERS_BY_OPERATION_SQL: &str =
    include_str!("../queries/legacy_membership_actions/previous_members_by_operation.sql");
const FINAL_MEMBERS_BY_OPERATION_SQL: &str =
    include_str!("../queries/legacy_membership_actions/final_members_by_operation.sql");
const AUTHORITY_SUBJECTS_BY_OPERATION_SQL: &str =
    include_str!("../queries/legacy_membership_actions/authority_subjects_by_operation.sql");

const REMOVE_INVITE_ACTION: &str = "legacy.membership.remove_organization_invite";
const ADD_MEMBER_ACTION: &str = "legacy.membership.add_space_member";
const ADD_MEMBERS_ACTION: &str = "legacy.membership.add_space_members";
const BATCH_REMOVE_MEMBERS_ACTION: &str = "legacy.membership.batch_remove_space_members";
const REMOVE_MEMBER_ACTION: &str = "legacy.membership.remove_space_member";
const SET_MEMBERS_ACTION: &str = "legacy.membership.set_space_members";
const ORGANIZATION_SETTINGS_PATH: &str = "/dashboard/settings/organization";
const AUTHORITY_SENTINEL: &str = "frame_legacy_membership_authority_v1";
const TARGET_SENTINEL: &str = "frame_legacy_membership_target_v1";
const CONFLICT_SENTINEL: &str = "frame_legacy_membership_conflict_v1";
const CORRUPT_SENTINEL: &str = "frame_legacy_membership_corrupt_v1";
// JSON staging plus alias evidence keeps the worst-case set batch at 43
// statements and the full fresh invocation (discovery, operation/clock reads,
// batch, operation reload, and three receipt-evidence reads) at D1's 50-query
// Free-plan invocation ceiling.
const MAX_FRESH_SET_BATCH_STATEMENTS: usize = 43;

type AtomicResult<T> = Result<T, LegacyMembershipAtomicErrorV1>;

pub(crate) struct D1LegacyMembershipAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyMembershipAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyMembershipAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
    }

    async fn batch_results(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> AtomicResult<Vec<D1Result>> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyMembershipAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(results)
    }
}

fn map_d1_message(message: &str) -> LegacyMembershipAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyMembershipAtomicErrorV1::StaleAuthority
    } else if message.contains(TARGET_SENTINEL) {
        LegacyMembershipAtomicErrorV1::TargetMissing
    } else if message.contains(CONFLICT_SENTINEL) {
        LegacyMembershipAtomicErrorV1::Conflict
    } else if message.contains(CORRUPT_SENTINEL)
        || message.contains("frame_legacy_membership_receipt_immutable_v1")
        || message.contains("frame_legacy_membership_proof_immutable_v1")
        || message.contains("frame_legacy_membership_alias_immutable_v1")
    {
        LegacyMembershipAtomicErrorV1::Corrupt
    } else {
        LegacyMembershipAtomicErrorV1::Unavailable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    RemoveInvite,
    AddMember,
    AddMembers,
    BatchRemoveMembers,
    RemoveMember,
    SetMembers,
}

impl Action {
    const fn journal_name(self) -> &'static str {
        match self {
            Self::RemoveInvite => REMOVE_INVITE_ACTION,
            Self::AddMember => ADD_MEMBER_ACTION,
            Self::AddMembers => ADD_MEMBERS_ACTION,
            Self::BatchRemoveMembers => BATCH_REMOVE_MEMBERS_ACTION,
            Self::RemoveMember => REMOVE_MEMBER_ACTION,
            Self::SetMembers => SET_MEMBERS_ACTION,
        }
    }
}

#[derive(Debug, Clone)]
struct Scope {
    organization_id: String,
    actor_id: String,
    action: Action,
}

impl Scope {
    fn from_command(command: &LegacyMembershipCommandV1) -> AtomicResult<Self> {
        let active_organization = command.fence().authority().active_organization_id();
        let action = match command {
            LegacyMembershipCommandV1::RemoveOrganizationInvite {
                organization_id, ..
            } => {
                if organization_id != &active_organization {
                    return Err(LegacyMembershipAtomicErrorV1::CrossTenant);
                }
                Action::RemoveInvite
            }
            LegacyMembershipCommandV1::AddSpaceMember { .. } => Action::AddMember,
            LegacyMembershipCommandV1::AddSpaceMembers { .. } => Action::AddMembers,
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. } => Action::BatchRemoveMembers,
            LegacyMembershipCommandV1::RemoveSpaceMember { .. } => Action::RemoveMember,
            LegacyMembershipCommandV1::SetSpaceMembers { .. } => Action::SetMembers,
        };
        Ok(Self {
            organization_id: active_organization.to_string(),
            actor_id: command.fence().authority().actor_id().to_string(),
            action,
        })
    }

    fn with_actor(&self, actor_id: String) -> Self {
        Self {
            organization_id: self.organization_id.clone(),
            actor_id,
            action: self.action,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct InviteAuthorityRow {
    selection_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: Option<String>,
    membership_state: Option<String>,
    membership_revision: Option<i64>,
    membership_authority_version: Option<i64>,
    actor_authority: String,
}

type TenantAuthorityRow = InviteAuthorityRow;

#[derive(Debug, Clone, Deserialize)]
struct SpaceAuthorityRow {
    selection_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: Option<String>,
    membership_state: Option<String>,
    membership_revision: Option<i64>,
    membership_authority_version: Option<i64>,
    space_id: String,
    creator_id: String,
    space_revision: i64,
    space_authority_version: i64,
    space_membership_role: Option<String>,
    space_membership_state: Option<String>,
    space_membership_revision: Option<i64>,
    actor_authority: String,
}

#[derive(Debug, Clone)]
enum AuthoritySnapshot {
    Invite(InviteAuthorityRow),
    Tenant(TenantAuthorityRow),
    Space(SpaceAuthorityRow),
}

impl AuthoritySnapshot {
    fn actor_authority_code(&self) -> &str {
        match self {
            Self::Invite(row) | Self::Tenant(row) => &row.actor_authority,
            Self::Space(row) => &row.actor_authority,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    organization_id: String,
    actor_id: String,
    action: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    invite_id: Option<String>,
    space_id: Option<String>,
    creator_id: Option<String>,
    actor_authority: Option<String>,
    matching_before: Option<i64>,
    deleted_rows: Option<i64>,
    inserted_rows: Option<i64>,
    matching_after: Option<i64>,
    result_count: Option<i64>,
    invalidates_organization_invites: Option<i64>,
    invalidates_space_page: Option<i64>,
    invalidates_space_members: Option<i64>,
    bumps_authority_generation: Option<i64>,
    authority_subject_count: Option<i64>,
    revalidation_path: Option<String>,
    audit_count: i64,
    proof_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct PreviousMemberRow {
    user_id: String,
    legacy_user_id: String,
    legacy_member_id: String,
    mapped_member_id: String,
    role: String,
    state: String,
    revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct FinalMemberRow {
    user_id: String,
    legacy_user_id: String,
    legacy_member_id: String,
    mapped_member_id: String,
    role: String,
    ordinal: i64,
}

#[derive(Debug, Deserialize)]
struct MemberAliasTargetRow {
    mapped_member_id: String,
    legacy_member_id: String,
    legacy_user_id: String,
    space_id: String,
    user_id: String,
    removed_at_ms: Option<i64>,
    role: Option<String>,
    state: Option<String>,
    revision: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AuthoritySubjectRow {
    user_id: String,
    generation_before: i64,
    generation_after: i64,
}

#[derive(Debug, Deserialize)]
struct ConsumedProofRow {
    mutation_grant_id: String,
    session_id: String,
    actor_id: String,
}

#[derive(Debug, Clone, Copy)]
struct ConsumedProof {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl InviteAuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        validate_revisions(&[
            Some(self.selection_revision),
            Some(self.organization_revision),
            Some(self.organization_authority_version),
            self.membership_revision,
            self.membership_authority_version,
        ])?;
        validate_membership_shape(
            self.membership_role.as_deref(),
            self.membership_state.as_deref(),
            self.membership_revision,
            self.membership_authority_version,
        )?;
        if !matches!(
            self.actor_authority.as_str(),
            "organization_owner" | "organization_admin"
        ) {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        if self.actor_authority == "organization_admin"
            && (self.membership_role.as_deref() != Some("admin")
                || self.membership_state.as_deref() != Some("active"))
        {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn validate_tenant(&self) -> AtomicResult<()> {
        validate_revisions(&[
            Some(self.selection_revision),
            Some(self.organization_revision),
            Some(self.organization_authority_version),
            self.membership_revision,
            self.membership_authority_version,
        ])?;
        validate_membership_shape(
            self.membership_role.as_deref(),
            self.membership_state.as_deref(),
            self.membership_revision,
            self.membership_authority_version,
        )?;
        match self.actor_authority.as_str() {
            "organization_owner" => Ok(()),
            "organization_admin"
                if self.membership_role.as_deref() == Some("admin")
                    && self.membership_state.as_deref() == Some("active") =>
            {
                Ok(())
            }
            "active_organization_member" if self.membership_state.as_deref() == Some("active") => {
                Ok(())
            }
            _ => Err(LegacyMembershipAtomicErrorV1::Corrupt),
        }
    }
}

impl SpaceAuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        validate_revisions(&[
            Some(self.selection_revision),
            Some(self.organization_revision),
            Some(self.organization_authority_version),
            self.membership_revision,
            self.membership_authority_version,
            Some(self.space_revision),
            Some(self.space_authority_version),
            self.space_membership_revision,
        ])?;
        validate_membership_shape(
            self.membership_role.as_deref(),
            self.membership_state.as_deref(),
            self.membership_revision,
            self.membership_authority_version,
        )?;
        validate_space_membership_shape(
            self.space_membership_role.as_deref(),
            self.space_membership_state.as_deref(),
            self.space_membership_revision,
        )?;
        SpaceId::parse(&self.space_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        UserId::parse(&self.creator_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        match parse_actor_authority(&self.actor_authority)? {
            LegacyMembershipActorAuthorityV1::OrganizationAdmin
                if self.membership_role.as_deref() != Some("admin")
                    || self.membership_state.as_deref() != Some("active") =>
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            LegacyMembershipActorAuthorityV1::SpaceCreator
                if self.membership_state.as_deref() != Some("active") =>
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            LegacyMembershipActorAuthorityV1::SpaceManager
                if self.membership_state.as_deref() != Some("active")
                    || self.space_membership_role.as_deref() != Some("manager")
                    || self.space_membership_state.as_deref() != Some("active") =>
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            _ => {}
        }
        Ok(())
    }
}

fn validate_revisions(values: &[Option<i64>]) -> AtomicResult<()> {
    if values.iter().flatten().any(|value| *value < 0) {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    Ok(())
}

fn validate_membership_shape(
    role: Option<&str>,
    state: Option<&str>,
    revision: Option<i64>,
    authority_version: Option<i64>,
) -> AtomicResult<()> {
    let all_absent =
        role.is_none() && state.is_none() && revision.is_none() && authority_version.is_none();
    let all_present =
        role.is_some() && state.is_some() && revision.is_some() && authority_version.is_some();
    if !(all_absent || all_present)
        || role.is_some_and(|value| !matches!(value, "owner" | "admin" | "member" | "viewer"))
        || state.is_some_and(|value| !matches!(value, "active" | "suspended" | "removed"))
    {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    Ok(())
}

fn validate_space_membership_shape(
    role: Option<&str>,
    state: Option<&str>,
    revision: Option<i64>,
) -> AtomicResult<()> {
    let all_absent = role.is_none() && state.is_none() && revision.is_none();
    let all_present = role.is_some() && state.is_some() && revision.is_some();
    if !(all_absent || all_present)
        || role.is_some_and(|value| !matches!(value, "manager" | "contributor" | "viewer"))
        || state.is_some_and(|value| !matches!(value, "active" | "suspended" | "removed"))
    {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    Ok(())
}

fn parse_actor_authority(value: &str) -> AtomicResult<LegacyMembershipActorAuthorityV1> {
    match value {
        "organization_owner" => Ok(LegacyMembershipActorAuthorityV1::OrganizationOwner),
        "organization_admin" => Ok(LegacyMembershipActorAuthorityV1::OrganizationAdmin),
        "active_organization_member" => {
            Ok(LegacyMembershipActorAuthorityV1::ActiveOrganizationMember)
        }
        "space_creator" => Ok(LegacyMembershipActorAuthorityV1::SpaceCreator),
        "space_manager" => Ok(LegacyMembershipActorAuthorityV1::SpaceManager),
        _ => Err(LegacyMembershipAtomicErrorV1::Corrupt),
    }
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn js_opt(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

#[allow(clippy::cast_precision_loss)]
fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn number_opt(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, number)
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

fn digest_fields(domain: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn operation_key_digest(scope: &Scope, raw_key: &str) -> String {
    digest_fields(
        b"frame.legacy-membership.operation-key.v1\0",
        &[
            &scope.organization_id,
            &scope.actor_id,
            scope.action.journal_name(),
            raw_key,
        ],
    )
}

fn attempt_digest(command: &LegacyMembershipCommandV1) -> String {
    let scope = Scope::from_command(command).ok();
    let organization = scope.as_ref().map_or_else(
        || "cross-tenant".to_owned(),
        |scope| scope.organization_id.clone(),
    );
    let actor = command.fence().authority().actor_id().to_string();
    let action = scope
        .as_ref()
        .map_or("invalid", |scope| scope.action.journal_name());
    let mut digest = Sha256::new();
    digest.update(b"frame.legacy-membership.attempt.v1\0");
    for field in [&organization, &actor, action] {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    match command {
        LegacyMembershipCommandV1::RemoveOrganizationInvite { invite_id, .. } => {
            digest.update(invite_id.as_uuid().as_bytes());
        }
        LegacyMembershipCommandV1::AddSpaceMember {
            space_id, target, ..
        } => {
            digest.update(space_id.as_uuid().as_bytes());
            digest.update(target.user_id().as_uuid().as_bytes());
            digest.update(target.role().stable_code().as_bytes());
        }
        LegacyMembershipCommandV1::AddSpaceMembers {
            space_id,
            legacy_user_ids,
            submitted_members,
            ..
        } => {
            digest.update(space_id.as_uuid().as_bytes());
            digest.update(submitted_members.len().to_be_bytes());
            for (legacy_user_id, member) in legacy_user_ids.iter().zip(submitted_members) {
                digest.update(legacy_user_id.len().to_be_bytes());
                digest.update(legacy_user_id.as_bytes());
                digest.update(member.user_id().as_uuid().as_bytes());
                digest.update(member.role().stable_code().as_bytes());
            }
        }
        LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. } => {
            digest.update(member_ids.len().to_be_bytes());
            for member_id in member_ids {
                digest.update(member_id.legacy_id().len().to_be_bytes());
                digest.update(member_id.legacy_id().as_bytes());
            }
        }
        LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. } => {
            digest.update(member_id.legacy_id().as_bytes());
        }
        LegacyMembershipCommandV1::SetSpaceMembers {
            space_id,
            submitted_members,
            ..
        } => {
            digest.update(space_id.as_uuid().as_bytes());
            digest.update(submitted_members.len().to_be_bytes());
            for member in submitted_members {
                digest.update(member.user_id().as_uuid().as_bytes());
                digest.update(member.role().stable_code().as_bytes());
            }
        }
    }
    lower_hex(&digest.finalize())
}

fn database_role(target: LegacySpaceMemberTargetV1) -> &'static str {
    match target.role().frame_role() {
        SpaceRole::Manager => "manager",
        SpaceRole::Viewer => "viewer",
        SpaceRole::Contributor => unreachable!("legacy target cannot produce contributor"),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalMemberWire {
    user_id: String,
    legacy_user_id: String,
    legacy_member_id: String,
    mapped_member_id: String,
    role: &'static str,
}

fn final_members_json(
    legacy_user_ids: &[String],
    targets: &[LegacySpaceMemberTargetV1],
) -> AtomicResult<String> {
    if targets.len() > MAX_LEGACY_MEMBERSHIP_TARGETS || legacy_user_ids.len() != targets.len() {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    let wire = legacy_user_ids
        .iter()
        .zip(targets)
        .map(|(legacy_user_id, target)| {
            let legacy_user = LegacyCapNanoId::parse(legacy_user_id.clone())
                .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            if legacy_user.mapped_uuid().to_string() != target.user_id().to_string() {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let member_alias = new_legacy_member_alias()?;
            Ok(FinalMemberWire {
                user_id: target.user_id().to_string(),
                legacy_user_id: legacy_user_id.clone(),
                legacy_member_id: member_alias.0,
                mapped_member_id: member_alias.1,
                role: database_role(*target),
            })
        })
        .collect::<AtomicResult<Vec<_>>>()?;
    serde_json::to_string(&wire).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
}

fn new_legacy_member_alias() -> AtomicResult<(String, String)> {
    let compact = Uuid::now_v7().simple().to_string();
    let legacy_id = compact
        .get(..15)
        .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?
        .to_owned();
    let legacy = LegacyCapNanoId::parse(legacy_id.clone())
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    Ok((legacy_id, legacy.mapped_uuid().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberIdWire<'a> {
    legacy_member_id: &'a str,
    mapped_member_id: &'a str,
}

fn member_ids_json(member_ids: &[LegacySpaceMemberIdV1]) -> AtomicResult<String> {
    if member_ids.len() > MAX_LEGACY_MEMBERSHIP_TARGETS {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    serde_json::to_string(
        &member_ids
            .iter()
            .map(|member_id| MemberIdWire {
                legacy_member_id: member_id.legacy_id(),
                mapped_member_id: member_id.mapped_uuid(),
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
}

impl D1LegacyMembershipAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.remove(0).now_ms;
        TimestampMillis::new(now_ms).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        Ok(now_ms)
    }

    async fn invite_authority(&self, scope: &Scope) -> AtomicResult<InviteAuthorityRow> {
        let mut rows = self
            .rows::<InviteAuthorityRow>(
                INVITE_AUTHORITY_SNAPSHOT_SQL,
                vec![js(&scope.actor_id), js(&scope.organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyMembershipAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }

    async fn tenant_authority(&self, scope: &Scope) -> AtomicResult<TenantAuthorityRow> {
        let mut rows = self
            .rows::<TenantAuthorityRow>(
                TENANT_AUTHORITY_SNAPSHOT_SQL,
                vec![js(&scope.actor_id), js(&scope.organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyMembershipAtomicErrorV1::StaleAuthority)?;
        row.validate_tenant()?;
        Ok(row)
    }

    async fn removal_targets(
        &self,
        member_ids: &[LegacySpaceMemberIdV1],
    ) -> AtomicResult<Vec<MemberAliasTargetRow>> {
        let targets_json = member_ids_json(member_ids)?;
        let rows = self
            .rows::<MemberAliasTargetRow>(MEMBER_ALIAS_TARGETS_SQL, vec![js(&targets_json)])
            .await?;
        if rows.len() > MAX_LEGACY_MEMBERSHIP_TARGETS {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(rows)
    }

    async fn discover_removal(
        &self,
        command: &LegacyMembershipCommandV1,
        scope: &Scope,
        member_ids: &[LegacySpaceMemberIdV1],
        require_one: bool,
    ) -> AtomicResult<(AuthoritySnapshot, LegacyMembershipDiscoveredContextV1)> {
        let rows = self.removal_targets(member_ids).await?;
        let has_active = rows.iter().any(|row| {
            row.removed_at_ms.is_none()
                && row.role.is_some()
                && row.state.as_deref() == Some("active")
                && row.revision.is_some()
        });
        if !has_active {
            if require_one {
                let mut historical = rows
                    .into_iter()
                    .filter(|row| row.removed_at_ms.is_some())
                    .collect::<Vec<_>>();
                if historical.len() != 1 {
                    return Err(LegacyMembershipAtomicErrorV1::TargetMissing);
                }
                let row = historical
                    .pop()
                    .ok_or(LegacyMembershipAtomicErrorV1::TargetMissing)?;
                let space_id = SpaceId::parse(&row.space_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                let removed_member = removal_target_from_alias_row(row)?;
                let authority = self.space_authority(scope, space_id).await?;
                let creator_id = UserId::parse(&authority.creator_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                return Ok((
                    AuthoritySnapshot::Space(authority),
                    LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                        organization_id: command.fence().authority().active_organization_id(),
                        space_id,
                        creator_id,
                        removed_members: vec![removed_member],
                    },
                ));
            }
            let authority = self.tenant_authority(scope).await?;
            return Ok((
                AuthoritySnapshot::Tenant(authority),
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop {
                    organization_id: command.fence().authority().active_organization_id(),
                },
            ));
        }
        let active = rows
            .into_iter()
            .filter(|row| {
                row.removed_at_ms.is_none()
                    && row.role.is_some()
                    && row.state.as_deref() == Some("active")
                    && row.revision.is_some()
            })
            .collect::<Vec<_>>();
        let space_id = SpaceId::parse(&active[0].space_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        if active.iter().any(|row| row.space_id != active[0].space_id) {
            return Err(LegacyMembershipAtomicErrorV1::CrossTenant);
        }
        let authority = self.space_authority(scope, space_id).await?;
        let creator_id = UserId::parse(&authority.creator_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        let removed_members = active
            .into_iter()
            .map(removal_target_from_alias_row)
            .collect::<AtomicResult<Vec<_>>>()?;
        Ok((
            AuthoritySnapshot::Space(authority),
            LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                organization_id: command.fence().authority().active_organization_id(),
                space_id,
                creator_id,
                removed_members,
            },
        ))
    }

    async fn space_authority(
        &self,
        scope: &Scope,
        space_id: SpaceId,
    ) -> AtomicResult<SpaceAuthorityRow> {
        let mut rows = self
            .rows::<SpaceAuthorityRow>(
                SPACE_AUTHORITY_SNAPSHOT_SQL,
                vec![
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                    js(&space_id.to_string()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyMembershipAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        if row.space_id != space_id.to_string() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(row)
    }

    async fn discover(
        &self,
        command: &LegacyMembershipCommandV1,
        scope: &Scope,
    ) -> AtomicResult<(AuthoritySnapshot, LegacyMembershipDiscoveredContextV1)> {
        match command {
            LegacyMembershipCommandV1::RemoveOrganizationInvite {
                organization_id,
                invite_id,
                ..
            } => {
                let authority = self.invite_authority(scope).await?;
                Ok((
                    AuthoritySnapshot::Invite(authority),
                    LegacyMembershipDiscoveredContextV1::OrganizationInvite {
                        organization_id: *organization_id,
                        invite_id: *invite_id,
                    },
                ))
            }
            LegacyMembershipCommandV1::AddSpaceMember { space_id, .. } => {
                let authority = self.space_authority(scope, *space_id).await?;
                let creator_id = UserId::parse(&authority.creator_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                Ok((
                    AuthoritySnapshot::Space(authority),
                    LegacyMembershipDiscoveredContextV1::SpaceAdd {
                        organization_id: command.fence().authority().active_organization_id(),
                        space_id: *space_id,
                        creator_id,
                    },
                ))
            }
            LegacyMembershipCommandV1::AddSpaceMembers { space_id, .. } => {
                let authority = self.space_authority(scope, *space_id).await?;
                let creator_id = UserId::parse(&authority.creator_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                Ok((
                    AuthoritySnapshot::Space(authority),
                    LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                        organization_id: command.fence().authority().active_organization_id(),
                        space_id: *space_id,
                        creator_id,
                        previous_members: Vec::new(),
                    },
                ))
            }
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. } => {
                self.discover_removal(command, scope, member_ids, false)
                    .await
            }
            LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. } => {
                self.discover_removal(command, scope, std::slice::from_ref(member_id), true)
                    .await
            }
            LegacyMembershipCommandV1::SetSpaceMembers { space_id, .. } => {
                let authority = self.space_authority(scope, *space_id).await?;
                let creator_id = UserId::parse(&authority.creator_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                // The fingerprint depends on the effective creator-inclusive
                // final set, not on the previous set. D1 captures the latter
                // inside the committing batch and the durable receipt reloads it.
                Ok((
                    AuthoritySnapshot::Space(authority),
                    LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                        organization_id: command.fence().authority().active_organization_id(),
                        space_id: *space_id,
                        creator_id,
                        previous_member_ids: Vec::new(),
                    },
                ))
            }
        }
    }

    async fn operation(
        &self,
        scope: &Scope,
        key_digest: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(&scope.organization_id),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn previous_member_rows(
        &self,
        operation_id: &str,
    ) -> AtomicResult<Vec<PreviousMemberRow>> {
        let rows = self
            .rows::<PreviousMemberRow>(PREVIOUS_MEMBERS_BY_OPERATION_SQL, vec![js(operation_id)])
            .await?;
        if rows.len() > MAX_LEGACY_DISCOVERED_SPACE_MEMBERS {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let mut output = Vec::with_capacity(rows.len());
        let mut prior: Option<String> = None;
        for row in rows {
            if row.revision < 0
                || !matches!(row.role.as_str(), "manager" | "contributor" | "viewer")
                || !matches!(row.state.as_str(), "active" | "suspended" | "removed")
                || prior.as_ref().is_some_and(|value| value >= &row.user_id)
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let user_id =
                UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            let legacy_user = LegacyCapNanoId::parse(row.legacy_user_id.clone())
                .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            if legacy_user.mapped_uuid().to_string() != user_id.to_string() {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            LegacySpaceMemberIdV1::from_verified_database_row(
                row.legacy_member_id.clone(),
                row.mapped_member_id.clone(),
            )?;
            prior = Some(row.user_id.clone());
            output.push(row);
        }
        Ok(output)
    }

    async fn final_member_rows(&self, operation_id: &str) -> AtomicResult<Vec<FinalMemberRow>> {
        let rows = self
            .rows::<FinalMemberRow>(FINAL_MEMBERS_BY_OPERATION_SQL, vec![js(operation_id)])
            .await?;
        if rows.len() > MAX_LEGACY_MEMBERSHIP_TARGETS + 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let mut ordinals = std::collections::BTreeSet::new();
        for row in &rows {
            if !(0..=500).contains(&row.ordinal)
                || !ordinals.insert(row.ordinal)
                || !matches!(row.role.as_str(), "manager" | "viewer")
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let user_id =
                UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            let legacy_user = LegacyCapNanoId::parse(row.legacy_user_id.clone())
                .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            if legacy_user.mapped_uuid().to_string() != user_id.to_string() {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            LegacySpaceMemberIdV1::from_verified_database_row(
                row.legacy_member_id.clone(),
                row.mapped_member_id.clone(),
            )?;
        }
        Ok(rows)
    }

    async fn authority_subjects(&self, operation_id: &str) -> AtomicResult<Vec<UserId>> {
        let rows = self
            .rows::<AuthoritySubjectRow>(
                AUTHORITY_SUBJECTS_BY_OPERATION_SQL,
                vec![js(operation_id)],
            )
            .await?;
        let maximum = MAX_LEGACY_DISCOVERED_SPACE_MEMBERS + MAX_LEGACY_MEMBERSHIP_TARGETS + 1;
        if rows.len() > maximum {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let mut output = Vec::with_capacity(rows.len());
        let mut prior: Option<String> = None;
        for row in rows {
            if row.generation_before < 0
                || row.generation_after != row.generation_before + 1
                || prior.as_ref().is_some_and(|value| value >= &row.user_id)
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let user_id =
                UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            prior = Some(row.user_id);
            output.push(user_id);
        }
        Ok(output)
    }

    fn authority_assertion(
        &self,
        assertion_id: &str,
        scope: &Scope,
        authority: &AuthoritySnapshot,
    ) -> AtomicResult<D1PreparedStatement> {
        match authority {
            AuthoritySnapshot::Invite(row) => self.statement(
                INVITE_AUTHORITY_ASSERT_SQL,
                vec![
                    js(assertion_id),
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                    number(row.selection_revision),
                    number(row.organization_revision),
                    number(row.organization_authority_version),
                    js_opt(row.membership_role.as_deref()),
                    js_opt(row.membership_state.as_deref()),
                    number_opt(row.membership_revision),
                    number_opt(row.membership_authority_version),
                    js(&row.actor_authority),
                ],
            ),
            AuthoritySnapshot::Tenant(row) => self.statement(
                TENANT_AUTHORITY_ASSERT_SQL,
                vec![
                    js(assertion_id),
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                    number(row.selection_revision),
                    number(row.organization_revision),
                    number(row.organization_authority_version),
                    js_opt(row.membership_role.as_deref()),
                    js_opt(row.membership_state.as_deref()),
                    number_opt(row.membership_revision),
                    number_opt(row.membership_authority_version),
                    js(&row.actor_authority),
                ],
            ),
            AuthoritySnapshot::Space(row) => self.statement(
                SPACE_AUTHORITY_ASSERT_SQL,
                vec![
                    js(assertion_id),
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                    number(row.selection_revision),
                    number(row.organization_revision),
                    number(row.organization_authority_version),
                    js_opt(row.membership_role.as_deref()),
                    js_opt(row.membership_state.as_deref()),
                    number_opt(row.membership_revision),
                    number_opt(row.membership_authority_version),
                    js(&row.space_id),
                    js(&row.creator_id),
                    number(row.space_revision),
                    number(row.space_authority_version),
                    js_opt(row.space_membership_role.as_deref()),
                    js_opt(row.space_membership_state.as_deref()),
                    number_opt(row.space_membership_revision),
                    js(&row.actor_authority),
                ],
            ),
        }
    }

    fn browser_grant_assertion(
        &self,
        assertion_id: &str,
        fence: LegacyMembershipBrowserFenceV1,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_ASSERT_SQL,
            vec![
                js(assertion_id),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                number(now_ms),
            ],
        )
    }

    fn browser_grant_delete(
        &self,
        fence: LegacyMembershipBrowserFenceV1,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_DELETE_RETURNING_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
            ],
        )
    }

    fn changes_assertion(
        &self,
        assertion_id: &str,
        kind: &str,
        expected: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            CHANGES_ASSERT_SQL,
            vec![js(assertion_id), js(kind), number(expected)],
        )
    }

    fn proof_insert(
        &self,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PROOF_INSERT_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                js_opt(related_operation_id),
                js(&scope.organization_id),
                js(scope.action.journal_name()),
                js(request_digest),
                js(outcome),
                number(now_ms),
            ],
        )
    }

    fn cleanup(&self, assertion_id: &str) -> AtomicResult<D1PreparedStatement> {
        self.statement(ASSERTION_CLEANUP_SQL, vec![js(assertion_id)])
    }
}

fn decode_consumed_proof(
    result: &D1Result,
    fence: LegacyMembershipBrowserFenceV1,
) -> AtomicResult<ConsumedProof> {
    let mut rows = result
        .results::<ConsumedProofRow>()
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    if rows.len() != 1 {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    let row = rows.remove(0);
    let proof = ConsumedProof {
        mutation_grant_id: SessionMutationGrantId::parse(&row.mutation_grant_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?,
        session_id: SessionId::parse(&row.session_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?,
        actor_id: UserId::parse(&row.actor_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?,
    };
    if proof.mutation_grant_id != fence.mutation_grant_id()
        || proof.session_id != fence.session_id()
        || proof.actor_id != fence.actor_id()
    {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    Ok(proof)
}

impl OperationRow {
    fn validate_identity(&self, scope: &Scope, request_digest: &str) -> AtomicResult<()> {
        if Uuid::parse_str(&self.operation_id).is_err()
            || self.organization_id != scope.organization_id
            || self.actor_id != scope.actor_id
            || self.action != scope.action.journal_name()
            || self.request_digest != request_digest
            || !is_lower_sha256(&self.request_digest)
            || self.audit_count < 0
            || self.proof_count < 0
        {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn clean_claim(&self) -> bool {
        self.state == "claimed"
            && self.result_kind.is_none()
            && self.invite_id.is_none()
            && self.space_id.is_none()
            && self.creator_id.is_none()
            && self.actor_authority.is_none()
            && self.matching_before.is_none()
            && self.deleted_rows.is_none()
            && self.inserted_rows.is_none()
            && self.matching_after.is_none()
            && self.result_count.is_none()
            && self.invalidates_organization_invites.is_none()
            && self.invalidates_space_page.is_none()
            && self.invalidates_space_members.is_none()
            && self.bumps_authority_generation.is_none()
            && self.authority_subject_count.is_none()
            && self.revalidation_path.is_none()
            && self.audit_count == 0
            && self.proof_count == 0
    }

    fn require_complete(&self) -> AtomicResult<()> {
        if self.state != "complete" || self.audit_count != 1 || self.proof_count < 1 {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl D1LegacyMembershipAtomicPortV1<'_> {
    async fn build_receipt(
        &self,
        command: &LegacyMembershipCommandV1,
        scope: &Scope,
        operation: &OperationRow,
    ) -> AtomicResult<LegacyMembershipMutationReceiptV1> {
        operation.require_complete()?;
        let organization_id = OrganizationId::parse(&operation.organization_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        let actor_id = UserId::parse(&operation.actor_id)
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        let actor_authority = parse_actor_authority(
            operation
                .actor_authority
                .as_deref()
                .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
        )?;
        let previous_rows = self.previous_member_rows(&operation.operation_id).await?;
        let previous = previous_rows
            .iter()
            .map(|row| {
                UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
            })
            .collect::<AtomicResult<Vec<_>>>()?;
        let final_rows = self.final_member_rows(&operation.operation_id).await?;
        let mut final_members = final_rows
            .iter()
            .map(|row| {
                let user_id = UserId::parse(&row.user_id)
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                let role = match row.role.as_str() {
                    "manager" => SpaceRole::Manager,
                    "viewer" => SpaceRole::Viewer,
                    _ => return Err(LegacyMembershipAtomicErrorV1::Corrupt),
                };
                LegacySpaceMemberTargetV1::from_database_role(user_id, role)
            })
            .collect::<AtomicResult<Vec<_>>>()?;
        final_members.sort_by_key(|target| target.user_id().to_string());
        let authority_subjects = self.authority_subjects(&operation.operation_id).await?;
        let subject_count = required_usize(operation.authority_subject_count)?;
        if subject_count != authority_subjects.len() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }

        let matching_before = required_u32(operation.matching_before)?;
        let deleted_rows = required_u32(operation.deleted_rows)?;
        let inserted_rows = required_u32(operation.inserted_rows)?;
        let matching_after = required_u32(operation.matching_after)?;
        let mut active_targets = command
            .submitted_members()
            .iter()
            .map(|target| target.user_id())
            .collect::<Vec<_>>();

        let (result, context, mutation_postcondition, active_space_id, active_creator_id) =
            match (scope.action, command) {
                (
                    Action::RemoveInvite,
                    LegacyMembershipCommandV1::RemoveOrganizationInvite {
                        organization_id: command_organization,
                        invite_id: command_invite,
                        ..
                    },
                ) => {
                    let invite_id = OrganizationInviteId::parse(
                        operation
                            .invite_id
                            .as_deref()
                            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
                    )
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                    if operation.result_kind.as_deref() != Some("organization_invite_removed")
                        || invite_id != *command_invite
                        || organization_id != *command_organization
                        || !previous.is_empty()
                        || !final_members.is_empty()
                        || !authority_subjects.is_empty()
                        || operation.result_count.is_some()
                        || effect_tuple(operation)? != (1, 0, 0, 0)
                        || operation.revalidation_path.as_deref()
                            != Some(ORGANIZATION_SETTINGS_PATH)
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    (
                        LegacyMembershipMutationResultV1::InviteRemoved,
                        LegacyMembershipDiscoveredContextV1::OrganizationInvite {
                            organization_id,
                            invite_id,
                        },
                        LegacyMembershipMutationPostconditionV1::OrganizationInviteRemoved {
                            matching_before,
                            deleted_rows,
                            matching_after,
                        },
                        None,
                        None,
                    )
                }
                (
                    Action::AddMember,
                    LegacyMembershipCommandV1::AddSpaceMember {
                        space_id: command_space,
                        target,
                        ..
                    },
                ) => {
                    let (space_id, creator_id) = parse_space_context(operation)?;
                    if operation.result_kind.as_deref() != Some("space_member_added")
                        || space_id != *command_space
                        || !previous.is_empty()
                        || final_members.as_slice() != std::slice::from_ref(target)
                        || authority_subjects != vec![target.user_id()]
                        || operation.result_count.is_some()
                        || effect_tuple(operation)? != (0, 1, 1, 1)
                        || operation.revalidation_path.as_deref()
                            != Some(format!("/dashboard/spaces/{space_id}").as_str())
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    (
                        LegacyMembershipMutationResultV1::SpaceMemberAdded,
                        LegacyMembershipDiscoveredContextV1::SpaceAdd {
                            organization_id,
                            space_id,
                            creator_id,
                        },
                        LegacyMembershipMutationPostconditionV1::SpaceMemberInserted {
                            matching_before,
                            inserted_rows,
                            matching_after,
                            final_member: *target,
                        },
                        Some(space_id),
                        Some(creator_id),
                    )
                }
                (
                    Action::AddMembers,
                    LegacyMembershipCommandV1::AddSpaceMembers {
                        space_id: command_space,
                        legacy_user_ids,
                        submitted_members,
                        ..
                    },
                ) => {
                    let (space_id, creator_id) = parse_space_context(operation)?;
                    if legacy_user_ids.len() != submitted_members.len() {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    let previous_aliases = previous_rows
                        .iter()
                        .map(|row| {
                            let user_id = UserId::parse(&row.user_id)
                                .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                            LegacySpaceMemberUserAliasV1::from_verified_database_row(
                                row.legacy_user_id.clone(),
                                user_id,
                            )
                        })
                        .collect::<AtomicResult<Vec<_>>>()?;
                    let previous_ids = previous
                        .iter()
                        .map(ToString::to_string)
                        .collect::<std::collections::BTreeSet<_>>();
                    let added_pairs = legacy_user_ids
                        .iter()
                        .zip(submitted_members)
                        .filter(|(_, target)| !previous_ids.contains(&target.user_id().to_string()))
                        .collect::<Vec<_>>();
                    let added = added_pairs
                        .iter()
                        .map(|(legacy_user_id, _)| (*legacy_user_id).clone())
                        .collect::<Vec<_>>();
                    let already_members = if added.is_empty() {
                        legacy_user_ids.clone()
                    } else {
                        previous_rows
                            .iter()
                            .map(|row| row.legacy_user_id.clone())
                            .collect()
                    };
                    let added_targets = added_pairs
                        .iter()
                        .map(|(_, target)| **target)
                        .collect::<Vec<_>>();
                    let mut added_subjects = added_targets
                        .iter()
                        .map(|target| target.user_id())
                        .collect::<Vec<_>>();
                    added_subjects.sort_by_key(ToString::to_string);
                    let expected_flags = if added.is_empty() {
                        (0, 1, 1, 0)
                    } else {
                        (0, 1, 1, 1)
                    };
                    if operation.result_kind.as_deref() != Some("space_members_added")
                        || space_id != *command_space
                        || required_u32(operation.result_count)?
                            != u32::try_from(added.len())
                                .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?
                        || authority_subjects != added_subjects
                        || effect_tuple(operation)? != expected_flags
                        || operation.revalidation_path.as_deref()
                            != Some(format!("/dashboard/spaces/{space_id}").as_str())
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    (
                        LegacyMembershipMutationResultV1::SpaceMembersAdded {
                            added,
                            already_members: already_members.clone(),
                        },
                        LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                            organization_id,
                            space_id,
                            creator_id,
                            previous_members: previous_aliases,
                        },
                        LegacyMembershipMutationPostconditionV1::SpaceMembersInserted {
                            matching_before,
                            inserted_rows,
                            matching_after,
                            added_members: added_targets,
                            already_member_ids: already_members,
                        },
                        Some(space_id),
                        Some(creator_id),
                    )
                }
                (
                    Action::BatchRemoveMembers,
                    LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. },
                ) => {
                    let removed_members = removal_targets_from_rows(&previous_rows)?;
                    active_targets = removed_members
                        .iter()
                        .map(LegacySpaceMemberRemovalTargetV1::user_id)
                        .collect();
                    let no_match = removed_members.is_empty();
                    let removed_member_ids = if no_match {
                        Vec::new()
                    } else {
                        member_ids.clone()
                    };
                    let expected_result_count = u32::try_from(removed_member_ids.len())
                        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                    if operation.result_kind.as_deref() != Some("space_members_removed")
                        || required_u32(operation.result_count)? != expected_result_count
                        || !final_members.is_empty()
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    if no_match {
                        if operation.space_id.is_some()
                            || operation.creator_id.is_some()
                            || !authority_subjects.is_empty()
                            || effect_tuple(operation)? != (0, 0, 0, 0)
                            || operation.revalidation_path.as_deref() != Some("")
                        {
                            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                        }
                        (
                            LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                                removed_member_ids,
                            },
                            LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop {
                                organization_id,
                            },
                            LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                                matching_before,
                                deleted_rows,
                                matching_after,
                                removed_members,
                            },
                            None,
                            None,
                        )
                    } else {
                        let (space_id, creator_id) = parse_space_context(operation)?;
                        let removed_subjects = active_targets.clone();
                        if authority_subjects != removed_subjects
                            || effect_tuple(operation)? != (0, 1, 1, 1)
                            || operation.revalidation_path.as_deref()
                                != Some(format!("/dashboard/spaces/{space_id}").as_str())
                        {
                            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                        }
                        (
                            LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                                removed_member_ids,
                            },
                            LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                                organization_id,
                                space_id,
                                creator_id,
                                removed_members: removed_members.clone(),
                            },
                            LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                                matching_before,
                                deleted_rows,
                                matching_after,
                                removed_members,
                            },
                            Some(space_id),
                            Some(creator_id),
                        )
                    }
                }
                (
                    Action::RemoveMember,
                    LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. },
                ) => {
                    let removed_members = removal_targets_from_rows(&previous_rows)?;
                    active_targets = removed_members
                        .iter()
                        .map(LegacySpaceMemberRemovalTargetV1::user_id)
                        .collect();
                    let (space_id, creator_id) = parse_space_context(operation)?;
                    if operation.result_kind.as_deref() != Some("space_member_removed")
                        || removed_members.len() != 1
                        || removed_members[0].member_id() != member_id
                        || authority_subjects != active_targets
                        || !final_members.is_empty()
                        || operation.result_count.is_some()
                        || effect_tuple(operation)? != (0, 1, 1, 1)
                        || operation.revalidation_path.as_deref()
                            != Some(format!("/dashboard/spaces/{space_id}").as_str())
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    (
                        LegacyMembershipMutationResultV1::SpaceMemberRemoved,
                        LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                            organization_id,
                            space_id,
                            creator_id,
                            removed_members: removed_members.clone(),
                        },
                        LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                            matching_before,
                            deleted_rows,
                            matching_after,
                            removed_members,
                        },
                        Some(space_id),
                        Some(creator_id),
                    )
                }
                (
                    Action::SetMembers,
                    LegacyMembershipCommandV1::SetSpaceMembers {
                        space_id: command_space,
                        ..
                    },
                ) => {
                    let (space_id, creator_id) = parse_space_context(operation)?;
                    let count = required_u32(operation.result_count)?;
                    let expected_subjects = authority_subjects_for_set(
                        &previous,
                        command.submitted_members(),
                        creator_id,
                    );
                    if operation.result_kind.as_deref() != Some("space_members_set")
                        || space_id != *command_space
                        || authority_subjects != expected_subjects
                        || effect_tuple(operation)? != (0, 1, 1, 1)
                        || operation.revalidation_path.as_deref()
                            != Some(format!("/dashboard/spaces/{space_id}").as_str())
                    {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    (
                        LegacyMembershipMutationResultV1::SpaceMembersSet { count },
                        LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                            organization_id,
                            space_id,
                            creator_id,
                            previous_member_ids: previous,
                        },
                        LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
                            matching_before,
                            deleted_rows,
                            inserted_rows,
                            matching_after,
                            final_members,
                        },
                        Some(space_id),
                        Some(creator_id),
                    )
                }
                _ => return Err(LegacyMembershipAtomicErrorV1::Corrupt),
            };

        let authority_postcondition =
            LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
                organization_id,
                actor_id,
                actor_authority,
                active_space_id,
                active_targets,
                active_creator_id,
                authority_subjects.clone(),
                authority_subjects,
            )?;
        LegacyMembershipMutationReceiptV1::new(
            command,
            result,
            context,
            mutation_postcondition,
            authority_postcondition,
        )
    }
}

fn required_u32(value: Option<i64>) -> AtomicResult<u32> {
    u32::try_from(value.ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
}

fn required_usize(value: Option<i64>) -> AtomicResult<usize> {
    usize::try_from(value.ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
}

fn effect_tuple(operation: &OperationRow) -> AtomicResult<(i64, i64, i64, i64)> {
    Ok((
        operation
            .invalidates_organization_invites
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
        operation
            .invalidates_space_page
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
        operation
            .invalidates_space_members
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
        operation
            .bumps_authority_generation
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
    ))
}

fn parse_space_context(operation: &OperationRow) -> AtomicResult<(SpaceId, UserId)> {
    let space_id = SpaceId::parse(
        operation
            .space_id
            .as_deref()
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    let creator_id = UserId::parse(
        operation
            .creator_id
            .as_deref()
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    Ok((space_id, creator_id))
}

fn removal_targets_from_rows(
    rows: &[PreviousMemberRow],
) -> AtomicResult<Vec<LegacySpaceMemberRemovalTargetV1>> {
    rows.iter()
        .map(|row| {
            let member_id = LegacySpaceMemberIdV1::from_verified_database_row(
                row.legacy_member_id.clone(),
                row.mapped_member_id.clone(),
            )?;
            let user_id =
                UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
            Ok(LegacySpaceMemberRemovalTargetV1::from_verified_database_row(member_id, user_id))
        })
        .collect()
}

fn removal_target_from_alias_row(
    row: MemberAliasTargetRow,
) -> AtomicResult<LegacySpaceMemberRemovalTargetV1> {
    let member_id = LegacySpaceMemberIdV1::from_verified_database_row(
        row.legacy_member_id,
        row.mapped_member_id,
    )?;
    let user_id =
        UserId::parse(&row.user_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    let legacy_user = LegacyCapNanoId::parse(row.legacy_user_id)
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    if legacy_user.mapped_uuid().to_string() != user_id.to_string() {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    Ok(LegacySpaceMemberRemovalTargetV1::from_verified_database_row(member_id, user_id))
}

fn authority_subjects_for_set(
    previous: &[UserId],
    submitted: &[LegacySpaceMemberTargetV1],
    creator_id: UserId,
) -> Vec<UserId> {
    let mut subjects = BTreeMap::new();
    for user_id in previous {
        subjects.insert(user_id.to_string(), *user_id);
    }
    for target in submitted {
        subjects.insert(target.user_id().to_string(), target.user_id());
    }
    subjects.insert(creator_id.to_string(), creator_id);
    subjects.into_values().collect()
}

impl D1LegacyMembershipAtomicPortV1<'_> {
    async fn consume_only(
        &self,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
    ) -> AtomicResult<ConsumedProof> {
        let assertion_id = Uuid::now_v7().to_string();
        let now_ms = self.clock_now().await?;
        let mut statements = vec![self.browser_grant_assertion(&assertion_id, fence, now_ms)?];
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(&assertion_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            related_operation_id,
            request_digest,
            outcome,
            now_ms,
        )?);
        statements.push(self.changes_assertion(&assertion_id, "proof_journaled", 1)?);
        statements.push(self.cleanup(&assertion_id)?);
        let results = self.batch_results(statements).await?;
        decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
            fence,
        )
    }

    async fn reject_and_return<T>(
        &self,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
        error: LegacyMembershipAtomicErrorV1,
    ) -> AtomicResult<T> {
        self.consume_only(fence, scope, related_operation_id, request_digest, outcome)
            .await?;
        Err(error)
    }

    async fn consume_replay(
        &self,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        authority: &AuthoritySnapshot,
        operation_id: &str,
        request_digest: &str,
    ) -> AtomicResult<ConsumedProof> {
        let now_ms = self.clock_now().await?;
        let mut statements = vec![self.authority_assertion(operation_id, scope, authority)?];
        if let AuthoritySnapshot::Space(row) = authority {
            statements.push(self.statement(
                if matches!(
                    scope.action,
                    Action::BatchRemoveMembers | Action::RemoveMember
                ) {
                    REMOVED_TARGET_GRAPH_ASSERT_SQL
                } else {
                    TARGET_GRAPH_ASSERT_SQL
                },
                vec![js(operation_id), js(&scope.organization_id)],
            )?);
            statements.push(self.statement(
                CREATOR_GRAPH_ASSERT_SQL,
                vec![
                    js(operation_id),
                    js(&scope.organization_id),
                    js(&row.creator_id),
                ],
            )?);
        }
        statements.push(self.browser_grant_assertion(operation_id, fence, now_ms)?);
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            Some(operation_id),
            request_digest,
            "replay",
            now_ms,
        )?);
        statements.push(self.changes_assertion(operation_id, "proof_journaled", 1)?);
        statements.push(self.cleanup(operation_id)?);
        let results = self.batch_results(statements).await?;
        decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
            fence,
        )
    }

    async fn existing_outcome(
        &self,
        command: &LegacyMembershipCommandV1,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        authority: &AuthoritySnapshot,
        operation: &OperationRow,
        request_digest: &str,
    ) -> AtomicResult<LegacyMembershipAtomicOutcomeV1> {
        if let Err(error) = operation.validate_identity(scope, request_digest) {
            let _ = self
                .consume_only(fence, scope, None, request_digest, "rejected")
                .await;
            return Err(error);
        }
        if operation.clean_claim() {
            return self
                .reject_and_return(
                    fence,
                    scope,
                    Some(&operation.operation_id),
                    request_digest,
                    "in_flight",
                    LegacyMembershipAtomicErrorV1::InFlight,
                )
                .await;
        }
        let receipt = match self.build_receipt(command, scope, operation).await {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = self
                    .consume_only(
                        fence,
                        scope,
                        Some(&operation.operation_id),
                        request_digest,
                        "rejected",
                    )
                    .await;
                return Err(error);
            }
        };
        if let Err(error) = self
            .consume_replay(
                fence,
                scope,
                authority,
                &operation.operation_id,
                request_digest,
            )
            .await
        {
            let _ = self
                .consume_only(
                    fence,
                    scope,
                    Some(&operation.operation_id),
                    request_digest,
                    "rejected",
                )
                .await;
            return Err(error);
        }
        Ok(LegacyMembershipAtomicOutcomeV1::Replay(receipt))
    }

    #[allow(clippy::too_many_arguments)]
    async fn reconcile(
        &self,
        command: &LegacyMembershipCommandV1,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        authority: &AuthoritySnapshot,
        key_digest: &str,
        request_digest: &str,
        original_error: LegacyMembershipAtomicErrorV1,
    ) -> AtomicResult<LegacyMembershipAtomicOutcomeV1> {
        match self.operation(scope, key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                self.existing_outcome(command, fence, scope, authority, &operation, request_digest)
                    .await
            }
            Ok(Some(operation)) => {
                if operation
                    .validate_identity(scope, &operation.request_digest)
                    .is_err()
                {
                    let _ = self
                        .consume_only(fence, scope, None, request_digest, "rejected")
                        .await;
                    return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                }
                self.reject_and_return(
                    fence,
                    scope,
                    Some(&operation.operation_id),
                    request_digest,
                    "conflict",
                    LegacyMembershipAtomicErrorV1::Conflict,
                )
                .await
            }
            Ok(None) => {
                let outcome = if original_error == LegacyMembershipAtomicErrorV1::Conflict {
                    "conflict"
                } else {
                    "rejected"
                };
                self.reject_and_return(fence, scope, None, request_digest, outcome, original_error)
                    .await
            }
            Err(_) => {
                let _ = self
                    .consume_only(fence, scope, None, request_digest, "rejected")
                    .await;
                Err(LegacyMembershipAtomicErrorV1::Unavailable)
            }
        }
    }
}

#[async_trait]
impl LegacyMembershipAtomicPortV1 for D1LegacyMembershipAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyMembershipCommandV1,
        browser_fence: &LegacyMembershipBrowserFenceV1,
    ) -> AtomicResult<LegacyMembershipAtomicOutcomeV1> {
        let fence = *browser_fence;
        let raw_digest = attempt_digest(command);
        let scope = match Scope::from_command(command) {
            Ok(scope) => scope,
            Err(error) => {
                let fallback = unchecked_scope(command).with_actor(fence.actor_id().to_string());
                let _ = self
                    .consume_only(fence, &fallback, None, &raw_digest, "rejected")
                    .await;
                return Err(error);
            }
        };
        if fence.actor_id().to_string() != scope.actor_id {
            let rejected_scope = scope.with_actor(fence.actor_id().to_string());
            let _ = self
                .consume_only(fence, &rejected_scope, None, &raw_digest, "rejected")
                .await;
            return Err(LegacyMembershipAtomicErrorV1::AccessDenied);
        }

        let (authority, fingerprint_context) = match self.discover(command, &scope).await {
            Ok(value) => value,
            Err(error) => {
                return self
                    .reject_and_return(fence, &scope, None, &raw_digest, "rejected", error)
                    .await;
            }
        };
        let request_digest = match command.request_fingerprint_for_context(&fingerprint_context) {
            Ok(fingerprint) => lower_hex(&fingerprint),
            Err(error) => {
                return self
                    .reject_and_return(fence, &scope, None, &raw_digest, "rejected", error)
                    .await;
            }
        };
        let key_digest = operation_key_digest(&scope, command.fence().idempotency_key().expose());

        match self.operation(&scope, &key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                return self
                    .existing_outcome(
                        command,
                        fence,
                        &scope,
                        &authority,
                        &operation,
                        &request_digest,
                    )
                    .await;
            }
            Ok(Some(operation)) => {
                if operation
                    .validate_identity(&scope, &operation.request_digest)
                    .is_err()
                {
                    let _ = self
                        .consume_only(fence, &scope, None, &request_digest, "rejected")
                        .await;
                    return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                }
                return self
                    .reject_and_return(
                        fence,
                        &scope,
                        Some(&operation.operation_id),
                        &request_digest,
                        "conflict",
                        LegacyMembershipAtomicErrorV1::Conflict,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self
                    .consume_only(fence, &scope, None, &request_digest, "rejected")
                    .await;
                return Err(error);
            }
        }

        match self
            .execute_fresh(
                command,
                fence,
                &scope,
                &authority,
                &key_digest,
                &request_digest,
            )
            .await
        {
            Ok(receipt) => Ok(LegacyMembershipAtomicOutcomeV1::Applied(receipt)),
            Err(error) => {
                self.reconcile(
                    command,
                    fence,
                    &scope,
                    &authority,
                    &key_digest,
                    &request_digest,
                    error,
                )
                .await
            }
        }
    }
}

fn unchecked_scope(command: &LegacyMembershipCommandV1) -> Scope {
    let action = match command {
        LegacyMembershipCommandV1::RemoveOrganizationInvite { .. } => Action::RemoveInvite,
        LegacyMembershipCommandV1::AddSpaceMember { .. } => Action::AddMember,
        LegacyMembershipCommandV1::AddSpaceMembers { .. } => Action::AddMembers,
        LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. } => Action::BatchRemoveMembers,
        LegacyMembershipCommandV1::RemoveSpaceMember { .. } => Action::RemoveMember,
        LegacyMembershipCommandV1::SetSpaceMembers { .. } => Action::SetMembers,
    };
    Scope {
        organization_id: command
            .fence()
            .authority()
            .active_organization_id()
            .to_string(),
        actor_id: command.fence().authority().actor_id().to_string(),
        action,
    }
}

impl D1LegacyMembershipAtomicPortV1<'_> {
    #[allow(clippy::too_many_arguments)]
    async fn execute_fresh(
        &self,
        command: &LegacyMembershipCommandV1,
        fence: LegacyMembershipBrowserFenceV1,
        scope: &Scope,
        authority: &AuthoritySnapshot,
        key_digest: &str,
        request_digest: &str,
    ) -> AtomicResult<LegacyMembershipMutationReceiptV1> {
        let now_ms = self.clock_now().await?;
        let operation_id = Uuid::now_v7().to_string();
        let audit_id = Uuid::now_v7().to_string();
        let principal_digest = digest_fields(
            b"frame.legacy-membership.principal.v1\0",
            &[&scope.actor_id],
        );
        let mutation_digest = digest_fields(
            b"frame.legacy-membership.subject.v1\0",
            &[
                &scope.organization_id,
                scope.action.journal_name(),
                request_digest,
            ],
        );
        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.organization_id),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                    js(request_digest),
                    number(now_ms),
                ],
            )?,
            self.authority_assertion(&operation_id, scope, authority)?,
            self.browser_grant_assertion(&operation_id, fence, now_ms)?,
        ];

        let delete_index = match command {
            LegacyMembershipCommandV1::RemoveOrganizationInvite {
                organization_id,
                invite_id,
                ..
            } => {
                statements.push(self.statement(
                    INVITE_TARGET_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&invite_id.to_string()),
                        js(&organization_id.to_string()),
                    ],
                )?);
                let delete_index = statements.len();
                statements.push(self.browser_grant_delete(fence)?);
                statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
                statements.push(self.statement(
                    INVITE_DELETE_SQL,
                    vec![js(&invite_id.to_string()), js(&organization_id.to_string())],
                )?);
                statements.push(self.changes_assertion(&operation_id, "mutation_rows", 1)?);
                statements.push(self.statement(
                    INVITE_POSTCONDITION_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&invite_id.to_string()),
                        js(&organization_id.to_string()),
                    ],
                )?);
                statements.push(self.statement(
                    RECEIPT_INSERT_INVITE_SQL,
                    vec![
                        js(&operation_id),
                        js(&invite_id.to_string()),
                        js(authority.actor_authority_code()),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&operation_id, "receipt_inserted", 1)?);
                statements.push(self.effect_insert(
                    &operation_id,
                    scope,
                    None,
                    (1, 0, 0, 0),
                    ORGANIZATION_SETTINGS_PATH,
                    now_ms,
                )?);
                statements.push(self.changes_assertion(&operation_id, "effect_inserted", 1)?);
                delete_index
            }
            LegacyMembershipCommandV1::AddSpaceMember {
                space_id,
                legacy_user_id,
                target,
                ..
            } => {
                let creator_id = required_space_creator(authority)?;
                let targets_json = final_members_json(
                    std::slice::from_ref(legacy_user_id),
                    std::slice::from_ref(target),
                )?;
                statements.push(self.statement(
                    FINAL_MEMBERS_INSERT_SQL,
                    vec![js(&operation_id), js(&targets_json)],
                )?);
                self.push_space_graph_assertions(
                    &mut statements,
                    &operation_id,
                    scope,
                    &creator_id,
                )?;
                statements.push(self.statement(
                    ADD_ABSENT_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&space_id.to_string()),
                        js(&target.user_id().to_string()),
                    ],
                )?);
                let delete_index = statements.len();
                statements.push(self.browser_grant_delete(fence)?);
                statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
                statements.push(self.statement(
                    ADD_INSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements.push(self.changes_assertion(&operation_id, "members_inserted", 1)?);
                statements.push(self.statement(
                    MEMBER_ALIAS_INSERT_ADDED_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements.push(self.changes_assertion(&operation_id, "aliases_inserted", 1)?);
                statements.push(self.statement(
                    MEMBER_ALIAS_ADDED_POSTCONDITION_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements.push(self.statement(
                    ADD_POSTCONDITION_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements.push(self.statement(
                    OUT_OF_SCOPE_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                self.push_authority_side_effects(&mut statements, &operation_id, scope, now_ms)?;
                statements.push(self.statement(
                    RECEIPT_INSERT_ADD_SQL,
                    vec![
                        js(&operation_id),
                        js(&space_id.to_string()),
                        js(&creator_id),
                        js(authority.actor_authority_code()),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&operation_id, "receipt_inserted", 1)?);
                let path = format!("/dashboard/spaces/{space_id}");
                statements.push(self.effect_insert(
                    &operation_id,
                    scope,
                    Some(&space_id.to_string()),
                    (0, 1, 1, 1),
                    &path,
                    now_ms,
                )?);
                statements.push(self.changes_assertion(&operation_id, "effect_inserted", 1)?);
                delete_index
            }
            LegacyMembershipCommandV1::AddSpaceMembers {
                space_id,
                legacy_user_ids,
                submitted_members,
                ..
            } => {
                let creator_id = required_space_creator(authority)?;
                let targets_json = final_members_json(legacy_user_ids, submitted_members)?;
                statements.push(self.statement(
                    BULK_ADD_DUPLICATE_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&targets_json),
                        js(&space_id.to_string()),
                    ],
                )?);
                statements.push(self.statement(
                    FINAL_MEMBERS_INSERT_SQL,
                    vec![js(&operation_id), js(&targets_json)],
                )?);
                self.push_space_graph_assertions(
                    &mut statements,
                    &operation_id,
                    scope,
                    &creator_id,
                )?;
                statements.push(self.statement(
                    PREVIOUS_SNAPSHOT_INSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements
                    .push(self.statement(PREVIOUS_BOUND_ASSERT_SQL, vec![js(&operation_id)])?);
                statements.push(self.statement(
                    PREVIOUS_ALIASES_COMPLETE_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                let delete_index = statements.len();
                statements.push(self.browser_grant_delete(fence)?);
                statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
                statements.push(self.statement(
                    BULK_ADD_INSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements
                    .push(self.statement(BULK_ADDED_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?);
                statements.push(self.statement(
                    MEMBER_ALIAS_INSERT_ADDED_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements.push(
                    self.statement(ALIASES_ADDED_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?,
                );
                statements.push(self.statement(
                    BULK_ADD_POSTCONDITION_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements.push(self.statement(
                    MEMBER_ALIAS_ADDED_POSTCONDITION_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements.push(self.statement(
                    OUT_OF_SCOPE_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                self.push_authority_side_effects_with(
                    &mut statements,
                    &operation_id,
                    scope,
                    now_ms,
                    AUTHORITY_SUBJECT_INSERT_ADDED_SQL,
                )?;
                statements.push(self.statement(
                    RECEIPT_INSERT_BULK_ADD_SQL,
                    vec![
                        js(&operation_id),
                        js(&space_id.to_string()),
                        js(&creator_id),
                        js(authority.actor_authority_code()),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&operation_id, "receipt_inserted", 1)?);
                let path = format!("/dashboard/spaces/{space_id}");
                statements.push(self.statement(
                    EFFECT_INSERT_BULK_ADD_SQL,
                    vec![
                        js(&operation_id),
                        js(&scope.organization_id),
                        js(&space_id.to_string()),
                        js(&path),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&operation_id, "effect_inserted", 1)?);
                delete_index
            }
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. } => self
                .push_removal_mutation(
                    &mut statements,
                    &operation_id,
                    scope,
                    authority,
                    fence,
                    member_ids,
                    false,
                    now_ms,
                )?,
            LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. } => self
                .push_removal_mutation(
                    &mut statements,
                    &operation_id,
                    scope,
                    authority,
                    fence,
                    std::slice::from_ref(member_id),
                    true,
                    now_ms,
                )?,
            LegacyMembershipCommandV1::SetSpaceMembers {
                space_id,
                legacy_user_ids,
                submitted_members,
                ..
            } => {
                let creator_id = required_space_creator(authority)?;
                let targets_json = final_members_json(legacy_user_ids, submitted_members)?;
                let creator_alias = new_legacy_member_alias()?;
                statements.push(self.statement(
                    FINAL_MEMBERS_INSERT_SQL,
                    vec![js(&operation_id), js(&targets_json)],
                )?);
                statements.push(self.statement(
                    FINAL_CREATOR_UPSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&creator_id),
                        js(&space_id.to_string()),
                        js(&creator_alias.0),
                        js(&creator_alias.1),
                    ],
                )?);
                self.push_space_graph_assertions(
                    &mut statements,
                    &operation_id,
                    scope,
                    &creator_id,
                )?;
                statements.push(self.statement(
                    PREVIOUS_SNAPSHOT_INSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements
                    .push(self.statement(PREVIOUS_BOUND_ASSERT_SQL, vec![js(&operation_id)])?);
                statements.push(self.statement(
                    PREVIOUS_ALIASES_COMPLETE_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                let delete_index = statements.len();
                statements.push(self.browser_grant_delete(fence)?);
                statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
                statements.push(self.statement(
                    MEMBER_ALIAS_REMOVE_PREVIOUS_SQL,
                    vec![js(&operation_id), number(now_ms)],
                )?);
                statements.push(
                    self.statement(ALIASES_PREVIOUS_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?,
                );
                statements.push(self.statement(SET_DELETE_SQL, vec![js(&space_id.to_string())])?);
                statements
                    .push(self.statement(PREVIOUS_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?);
                statements.push(self.statement(
                    SET_INSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements.push(self.statement(FINAL_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?);
                statements.push(self.statement(
                    MEMBER_ALIAS_INSERT_ALL_SQL,
                    vec![js(&operation_id), js(&space_id.to_string()), number(now_ms)],
                )?);
                statements.push(
                    self.statement(ALIASES_FINAL_CHANGES_ASSERT_SQL, vec![js(&operation_id)])?,
                );
                statements.push(self.statement(
                    MEMBER_ALIAS_POSTCONDITION_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                statements.push(self.statement(
                    SET_POSTCONDITION_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&space_id.to_string()),
                        js(&creator_id),
                    ],
                )?);
                statements.push(self.statement(
                    OUT_OF_SCOPE_ASSERT_SQL,
                    vec![js(&operation_id), js(&space_id.to_string())],
                )?);
                self.push_authority_side_effects(&mut statements, &operation_id, scope, now_ms)?;
                statements.push(self.statement(
                    RECEIPT_INSERT_SET_SQL,
                    vec![
                        js(&operation_id),
                        js(&space_id.to_string()),
                        js(&creator_id),
                        js(authority.actor_authority_code()),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&operation_id, "receipt_inserted", 1)?);
                let path = format!("/dashboard/spaces/{space_id}");
                statements.push(self.effect_insert(
                    &operation_id,
                    scope,
                    Some(&space_id.to_string()),
                    (0, 1, 1, 1),
                    &path,
                    now_ms,
                )?);
                statements.push(self.changes_assertion(&operation_id, "effect_inserted", 1)?);
                delete_index
            }
        };

        statements.push(self.statement(
            AUDIT_INSERT_SQL,
            vec![
                js(&audit_id),
                js(&operation_id),
                js(&scope.organization_id),
                js(&scope.actor_id),
                js(scope.action.journal_name()),
                js(&principal_digest),
                js(&mutation_digest),
                number(now_ms),
            ],
        )?);
        statements.push(self.changes_assertion(&operation_id, "audit_inserted", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            Some(&operation_id),
            request_digest,
            "applied",
            now_ms,
        )?);
        statements.push(self.changes_assertion(&operation_id, "proof_journaled", 1)?);
        statements.push(self.statement(
            OPERATION_COMPLETE_SQL,
            vec![js(&operation_id), number(now_ms)],
        )?);
        statements.push(self.changes_assertion(&operation_id, "operation_complete", 1)?);
        statements.push(self.statement(
            DURABLE_RECEIPT_ASSERT_SQL,
            vec![
                js(&operation_id),
                js(&scope.organization_id),
                js(&scope.actor_id),
                js(scope.action.journal_name()),
                js(request_digest),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js("applied"),
            ],
        )?);
        statements.push(self.cleanup(&operation_id)?);

        if scope.action == Action::SetMembers && statements.len() > MAX_FRESH_SET_BATCH_STATEMENTS {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }

        let results = self.batch_results(statements).await?;
        decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?,
            fence,
        )?;
        let operation = self
            .operation(scope, key_digest)
            .await?
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?;
        operation.validate_identity(scope, request_digest)?;
        self.build_receipt(command, scope, &operation).await
    }

    #[allow(clippy::too_many_arguments)]
    fn push_removal_mutation(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        scope: &Scope,
        authority: &AuthoritySnapshot,
        fence: LegacyMembershipBrowserFenceV1,
        member_ids: &[LegacySpaceMemberIdV1],
        single: bool,
        now_ms: i64,
    ) -> AtomicResult<usize> {
        let targets_json = member_ids_json(member_ids)?;
        if let AuthoritySnapshot::Tenant(_) = authority {
            if single {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            statements.push(self.statement(
                REMOVAL_NO_MATCH_ASSERT_SQL,
                vec![js(operation_id), js(&targets_json)],
            )?);
            let delete_index = statements.len();
            statements.push(self.browser_grant_delete(fence)?);
            statements.push(self.changes_assertion(operation_id, "grant_consumed", 1)?);
            statements.push(self.statement(
                RECEIPT_INSERT_BATCH_REMOVE_SQL,
                vec![
                    js(operation_id),
                    JsValue::NULL,
                    JsValue::NULL,
                    js(authority.actor_authority_code()),
                    number(0),
                    number(now_ms),
                ],
            )?);
            statements.push(self.changes_assertion(operation_id, "receipt_inserted", 1)?);
            statements.push(self.effect_insert(
                operation_id,
                scope,
                None,
                (0, 0, 0, 0),
                "",
                now_ms,
            )?);
            statements.push(self.changes_assertion(operation_id, "effect_inserted", 1)?);
            return Ok(delete_index);
        }
        let creator_id = required_space_creator(authority)?;
        let AuthoritySnapshot::Space(row) = authority else {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        };
        let space_id = &row.space_id;
        statements.push(self.statement(
            REMOVAL_TARGETS_INSERT_SQL,
            vec![js(operation_id), js(&targets_json), js(space_id)],
        )?);
        statements.push(self.statement(
            REMOVAL_TARGETS_ASSERT_SQL,
            vec![js(operation_id), number(1)],
        )?);
        statements.push(self.statement(
            REMOVED_TARGET_GRAPH_ASSERT_SQL,
            vec![js(operation_id), js(&scope.organization_id)],
        )?);
        statements.push(self.statement(
            REMOVAL_CREATOR_ASSERT_SQL,
            vec![js(operation_id), js(&creator_id)],
        )?);
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(operation_id, "grant_consumed", 1)?);
        statements.push(self.statement(
            MEMBER_ALIAS_REMOVE_PREVIOUS_SQL,
            vec![js(operation_id), number(now_ms)],
        )?);
        statements
            .push(self.statement(ALIASES_PREVIOUS_CHANGES_ASSERT_SQL, vec![js(operation_id)])?);
        statements.push(self.statement(REMOVAL_DELETE_SQL, vec![js(operation_id), js(space_id)])?);
        statements.push(self.statement(REMOVAL_CHANGES_ASSERT_SQL, vec![js(operation_id)])?);
        statements.push(self.statement(
            REMOVAL_POSTCONDITION_ASSERT_SQL,
            vec![js(operation_id), js(space_id)],
        )?);
        self.push_authority_side_effects_with(
            statements,
            operation_id,
            scope,
            now_ms,
            AUTHORITY_SUBJECT_INSERT_REMOVED_SQL,
        )?;
        if single {
            statements.push(self.statement(
                RECEIPT_INSERT_REMOVE_MEMBER_SQL,
                vec![
                    js(operation_id),
                    js(space_id),
                    js(&creator_id),
                    js(authority.actor_authority_code()),
                    number(now_ms),
                ],
            )?);
        } else {
            statements.push(self.statement(
                RECEIPT_INSERT_BATCH_REMOVE_SQL,
                vec![
                    js(operation_id),
                    js(space_id),
                    js(&creator_id),
                    js(authority.actor_authority_code()),
                    number(
                        i64::try_from(member_ids.len())
                            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?,
                    ),
                    number(now_ms),
                ],
            )?);
        }
        statements.push(self.changes_assertion(operation_id, "receipt_inserted", 1)?);
        let path = format!("/dashboard/spaces/{space_id}");
        statements.push(self.effect_insert(
            operation_id,
            scope,
            Some(space_id),
            (0, 1, 1, 1),
            &path,
            now_ms,
        )?);
        statements.push(self.changes_assertion(operation_id, "effect_inserted", 1)?);
        Ok(delete_index)
    }

    fn push_space_graph_assertions(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        scope: &Scope,
        creator_id: &str,
    ) -> AtomicResult<()> {
        statements.push(self.statement(
            TARGET_GRAPH_ASSERT_SQL,
            vec![js(operation_id), js(&scope.organization_id)],
        )?);
        statements.push(self.statement(
            CREATOR_GRAPH_ASSERT_SQL,
            vec![js(operation_id), js(&scope.organization_id), js(creator_id)],
        )?);
        Ok(())
    }

    fn push_authority_side_effects(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        scope: &Scope,
        now_ms: i64,
    ) -> AtomicResult<()> {
        self.push_authority_side_effects_with(
            statements,
            operation_id,
            scope,
            now_ms,
            AUTHORITY_SUBJECT_INSERT_SQL,
        )
    }

    fn push_authority_side_effects_with(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        scope: &Scope,
        now_ms: i64,
        subject_insert_sql: &str,
    ) -> AtomicResult<()> {
        statements.push(self.statement(
            subject_insert_sql,
            vec![js(operation_id), js(&scope.organization_id)],
        )?);
        statements.push(self.statement(
            AUTHORITY_GENERATION_UPSERT_SQL,
            vec![js(operation_id), js(&scope.organization_id), number(now_ms)],
        )?);
        statements.push(self.statement(
            AUTHORITY_GENERATION_CHANGES_ASSERT_SQL,
            vec![js(operation_id)],
        )?);
        statements.push(self.statement(
            AUTHORITY_GENERATION_POSTCONDITION_ASSERT_SQL,
            vec![js(operation_id), js(&scope.organization_id)],
        )?);
        statements.push(self.statement(REVOKED_GRANT_SNAPSHOT_INSERT_SQL, vec![js(operation_id)])?);
        statements.push(self.statement(REVOKE_GRANTS_SQL, vec![js(operation_id)])?);
        statements.push(self.statement(REVOKED_GRANT_CHANGES_ASSERT_SQL, vec![js(operation_id)])?);
        statements.push(self.statement(
            REVOKED_GRANT_POSTCONDITION_ASSERT_SQL,
            vec![js(operation_id)],
        )?);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn effect_insert(
        &self,
        operation_id: &str,
        scope: &Scope,
        space_id: Option<&str>,
        flags: (i64, i64, i64, i64),
        path: &str,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            EFFECT_INSERT_SQL,
            vec![
                js(operation_id),
                js(&scope.organization_id),
                js_opt(space_id),
                number(flags.0),
                number(flags.1),
                number(flags.2),
                number(flags.3),
                js(path),
                number(now_ms),
            ],
        )
    }
}

fn required_space_creator(authority: &AuthoritySnapshot) -> AtomicResult<String> {
    let AuthoritySnapshot::Space(row) = authority else {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    };
    UserId::parse(&row.creator_id).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    Ok(row.creator_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(value: &str) -> UserId {
        UserId::parse(value).expect("valid user")
    }

    #[test]
    fn operation_key_is_tenant_actor_action_and_secret_scoped() {
        let base = Scope {
            organization_id: "018f0000-0000-7000-8000-000000000001".into(),
            actor_id: "018f0000-0000-7000-8000-000000000002".into(),
            action: Action::SetMembers,
        };
        let mut other_actor = base.clone();
        other_actor.actor_id = "018f0000-0000-7000-8000-000000000003".into();
        let mut other_tenant = base.clone();
        other_tenant.organization_id = "018f0000-0000-7000-8000-000000000004".into();
        let mut other_action = base.clone();
        other_action.action = Action::AddMember;
        let digest = operation_key_digest(&base, "browser-secret-0001");
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("browser-secret-0001"));
        assert_ne!(
            digest,
            operation_key_digest(&other_actor, "browser-secret-0001")
        );
        assert_ne!(
            digest,
            operation_key_digest(&other_tenant, "browser-secret-0001")
        );
        assert_ne!(
            digest,
            operation_key_digest(&other_action, "browser-secret-0001")
        );
        assert_ne!(digest, operation_key_digest(&base, "browser-secret-0002"));
    }

    #[test]
    fn set_authority_subjects_are_the_exact_prior_final_creator_union() {
        let prior_one = user("018f0000-0000-7000-8000-000000000011");
        let prior_two = user("018f0000-0000-7000-8000-000000000012");
        let submitted = user("018f0000-0000-7000-8000-000000000013");
        let creator = user("018f0000-0000-7000-8000-000000000014");
        let target = LegacySpaceMemberTargetV1::from_database_role(submitted, SpaceRole::Viewer)
            .expect("legacy target");
        let subjects =
            authority_subjects_for_set(&[prior_two, prior_one, prior_two], &[target], creator);
        assert_eq!(subjects, vec![prior_one, prior_two, submitted, creator]);
    }

    #[test]
    fn authority_shapes_fail_closed_without_exact_admin_or_manager_rows() {
        let invalid_admin = InviteAuthorityRow {
            selection_revision: 0,
            organization_revision: 0,
            organization_authority_version: 0,
            membership_role: Some("member".into()),
            membership_state: Some("active".into()),
            membership_revision: Some(0),
            membership_authority_version: Some(0),
            actor_authority: "organization_admin".into(),
        };
        assert_eq!(
            invalid_admin.validate(),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let invalid_manager = SpaceAuthorityRow {
            selection_revision: 0,
            organization_revision: 0,
            organization_authority_version: 0,
            membership_role: Some("member".into()),
            membership_state: Some("active".into()),
            membership_revision: Some(0),
            membership_authority_version: Some(0),
            space_id: "018f0000-0000-7000-8000-000000000021".into(),
            creator_id: "018f0000-0000-7000-8000-000000000022".into(),
            space_revision: 0,
            space_authority_version: 0,
            space_membership_role: Some("contributor".into()),
            space_membership_state: Some("active".into()),
            space_membership_revision: Some(0),
            actor_authority: "space_manager".into(),
        };
        assert_eq!(
            invalid_manager.validate(),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn assertion_sentinels_map_to_non_disclosing_typed_failures() {
        assert_eq!(
            map_d1_message(AUTHORITY_SENTINEL),
            LegacyMembershipAtomicErrorV1::StaleAuthority
        );
        assert_eq!(
            map_d1_message(TARGET_SENTINEL),
            LegacyMembershipAtomicErrorV1::TargetMissing
        );
        assert_eq!(
            map_d1_message(CONFLICT_SENTINEL),
            LegacyMembershipAtomicErrorV1::Conflict
        );
        assert_eq!(
            map_d1_message(CORRUPT_SENTINEL),
            LegacyMembershipAtomicErrorV1::Corrupt
        );
        assert_eq!(
            map_d1_message("transport"),
            LegacyMembershipAtomicErrorV1::Unavailable
        );
    }

    #[test]
    fn sql_surface_is_bounded_scoped_creator_forcing_and_proof_returning() {
        for token in [
            "operation.organization_id = ?1",
            "operation.actor_id = ?2",
            "operation.action = ?3",
            "LIMIT 2",
        ] {
            assert!(OPERATION_BY_KEY_SQL.contains(token));
        }
        assert!(PREVIOUS_SNAPSHOT_INSERT_SQL.contains("LIMIT 100001"));
        assert!(PREVIOUS_MEMBERS_BY_OPERATION_SQL.contains("LIMIT 100001"));
        assert!(FINAL_MEMBERS_BY_OPERATION_SQL.contains("LIMIT 502"));
        assert!(AUTHORITY_SUBJECTS_BY_OPERATION_SQL.contains("LIMIT 100502"));
        assert!(BROWSER_GRANT_DELETE_RETURNING_SQL.contains("RETURNING id AS mutation_grant_id"));
        assert!(TARGET_GRAPH_ASSERT_SQL.contains("organization.id = ?2"));
        assert!(AUTHORITY_SUBJECT_INSERT_SQL.contains("UNION"));
        assert!(AUTHORITY_SUBJECT_INSERT_SQL.contains("generation_before"));
        assert!(FINAL_MEMBERS_INSERT_SQL.contains("FROM json_each(?2)"));
        assert_eq!(MAX_FRESH_SET_BATCH_STATEMENTS, 43);
        assert!(FINAL_CREATOR_UPSERT_SQL.contains("role = 'manager'"));
        assert!(SET_POSTCONDITION_ASSERT_SQL.contains("creator.role = 'manager'"));
        assert!(SET_DELETE_SQL.contains("WHERE space_id = ?1"));
        assert!(OUT_OF_SCOPE_ASSERT_SQL.contains("member.space_id <> ?2"));
        assert!(DURABLE_RECEIPT_ASSERT_SQL.contains("proof.mutation_grant_id = ?6"));
    }
}
