//! Closed, exact, token-gated local D1 organization conformance surface.

use frame_domain::{
    CollaborationName, FolderId, FolderRecord, IdempotencyKey, MembershipState, OrganizationAction,
    OrganizationAuthorityFence, OrganizationGraphFindingKind, OrganizationId,
    OrganizationOperationId, OrganizationRevision, OrganizationRole, OrganizationScope,
    OrganizationSettings, SecretDigest, SpaceId, TimestampMillis, TombstonePolicy, UserId,
};
use frame_ports::{
    AcceptOrganizationInviteCommand, ChangeOrganizationMemberCommand, CreateFolderCommand,
    MoveFolderCommand, OrganizationGraphAuditRequest, OrganizationMutationContext,
    OrganizationPortError, OrganizationReadRequest, OrganizationRepository,
    OrganizationWriteAuthority, RecoverOrganizationCommand, TombstoneOrganizationCommand,
    TransferOrganizationOwnershipCommand,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Method, Request, Response, Result};

use crate::{
    contracts::{API_SCHEMA_VERSION, constant_time_eq},
    organization_repository::D1OrganizationRepository,
};

const TOKEN_VARIABLE: &str = "FRAME_ORGANIZATION_REPOSITORY_CONFORMANCE_TOKEN";
const TOKEN_HEADER: &str = "x-frame-organization-repository-conformance-token";
const MAX_BODY_BYTES: usize = 128;
const NOW_MS: i64 = 1_700_200_000_000;

const ORG_SNAPSHOT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e140";
const ORG_INVITE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e141";
const ORG_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e142";
const ORG_FOLDER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e143";
const ORG_TOMBSTONE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e144";
const ORG_AUDIT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e145";
const ORG_AUTHORITY_RACE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e146";
const ORG_RETENTION: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e147";

const USER_SNAPSHOT_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a140";
const USER_INVITEE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a144";
const USER_OLD_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a142";
const USER_NEW_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a146";
const USER_FOLDER_MEMBER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a147";
const USER_TOMBSTONE_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a148";
const USER_SUPPORT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a149";
const USER_AUTHORITY_OWNER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a151";
const USER_AUTHORITY_MEMBER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a152";

const INVITE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f141";
const SPACE_FOLDER: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b143";
const FOLDER_ROOT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690c143";
const FOLDER_CHILD: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690c144";
const FOLDER_GRANDCHILD: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690c145";
const SPACE_AUTHORITY_RACE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b146";
const FOLDER_AUTHORITY_RACE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690c146";
const FOLDER_AFTER_DOWNGRADE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690c147";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceRequest {
    schema_version: u16,
    scenario: Scenario,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    SnapshotBoundary,
    InviteAcceptA,
    InviteAcceptB,
    InviteReplayAndMismatch,
    OwnershipTransfer,
    OwnerTargetRemoval,
    OwnershipInvariant,
    AuthorityMemberDowngrade,
    AuthorityFolderCreate,
    AuthorityPostDowngrade,
    AuthorityInvariant,
    FolderCycle,
    FolderMove,
    TombstoneLifecycle,
    RetentionExpiry,
    AuditRepair,
    AuditDenied,
}

impl Scenario {
    const fn name(self) -> &'static str {
        match self {
            Self::SnapshotBoundary => "snapshot_boundary",
            Self::InviteAcceptA => "invite_accept_a",
            Self::InviteAcceptB => "invite_accept_b",
            Self::InviteReplayAndMismatch => "invite_replay_and_mismatch",
            Self::OwnershipTransfer => "ownership_transfer",
            Self::OwnerTargetRemoval => "owner_target_removal",
            Self::OwnershipInvariant => "ownership_invariant",
            Self::AuthorityMemberDowngrade => "authority_member_downgrade",
            Self::AuthorityFolderCreate => "authority_folder_create",
            Self::AuthorityPostDowngrade => "authority_post_downgrade",
            Self::AuthorityInvariant => "authority_invariant",
            Self::FolderCycle => "folder_cycle",
            Self::FolderMove => "folder_move",
            Self::TombstoneLifecycle => "tombstone_lifecycle",
            Self::RetentionExpiry => "retention_expiry",
            Self::AuditRepair => "audit_repair",
            Self::AuditDenied => "audit_denied",
        }
    }
}

#[derive(Debug, Deserialize)]
struct OwnershipInvariantRow {
    owner_id: String,
    active_owner_count: i64,
    pointer_owner_count: i64,
}

#[derive(Debug, Deserialize)]
struct FolderInvariantRow {
    parent_id: Option<String>,
    depth: i64,
    self_edges: i64,
    cycle_edges: i64,
}

#[derive(Debug, Deserialize)]
struct AuthorityInvariantRow {
    role: String,
    state: String,
    session_version: i64,
    race_folders: i64,
    post_downgrade_folders: i64,
}

#[derive(Debug, Deserialize)]
struct TombstoneRow {
    tombstoned_at_ms: i64,
}

pub async fn response(mut request: Request, env: &Env) -> Result<Response> {
    if request.method() != Method::Post {
        return fixed_response(405, "method_not_allowed", None);
    }
    let expected = env
        .var(TOKEN_VARIABLE)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let supplied = request.headers().get(TOKEN_HEADER)?.unwrap_or_default();
    if !valid_token(&expected)
        || !valid_token(&supplied)
        || !constant_time_eq(expected.as_bytes(), supplied.as_bytes())
    {
        return fixed_response(404, "not_found", None);
    }
    let content_type = request.headers().get("content-type")?.unwrap_or_default();
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<usize>().ok());
    if content_type != "application/json"
        || content_length.is_none_or(|length| length == 0 || length > MAX_BODY_BYTES)
    {
        return fixed_response(400, "invalid_request", None);
    }
    let bytes = request.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return fixed_response(400, "invalid_request", None);
    }
    let body = match serde_json::from_slice::<ConformanceRequest>(&bytes) {
        Ok(body) if body.schema_version == API_SCHEMA_VERSION => body,
        _ => return fixed_response(400, "invalid_request", None),
    };
    let database = env.d1("DB")?;
    match run_scenario(&database, body.scenario).await {
        Ok(values) => fixed_response(
            200,
            "ok",
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": body.scenario.name(),
                "values": values,
            })),
        ),
        Err(_) => fixed_response(
            500,
            "organization_repository_conformance_failed",
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": body.scenario.name(),
            })),
        ),
    }
}

async fn run_scenario(database: &D1Database, scenario: Scenario) -> Result<Value> {
    let policy = TombstonePolicy::new(1_000, 10_000).map_err(|_| fixed_failure())?;
    let repository = D1OrganizationRepository::new(database, policy);
    match scenario {
        Scenario::SnapshotBoundary => snapshot_boundary(&repository).await,
        Scenario::InviteAcceptA => invite_accept(&repository, 1, 21).await,
        Scenario::InviteAcceptB => invite_accept(&repository, 2, 21).await,
        Scenario::InviteReplayAndMismatch => invite_replay_and_mismatch(&repository).await,
        Scenario::OwnershipTransfer => ownership_transfer(&repository).await,
        Scenario::OwnerTargetRemoval => owner_target_removal(&repository).await,
        Scenario::OwnershipInvariant => ownership_invariant(database).await,
        Scenario::AuthorityMemberDowngrade => authority_member_downgrade(&repository).await,
        Scenario::AuthorityFolderCreate => {
            authority_folder_create(&repository, FOLDER_AUTHORITY_RACE, false).await
        }
        Scenario::AuthorityPostDowngrade => {
            authority_folder_create(&repository, FOLDER_AFTER_DOWNGRADE, true).await
        }
        Scenario::AuthorityInvariant => authority_invariant(database).await,
        Scenario::FolderCycle => folder_cycle(&repository).await,
        Scenario::FolderMove => folder_move(database, &repository).await,
        Scenario::TombstoneLifecycle => tombstone_lifecycle(database, &repository).await,
        Scenario::RetentionExpiry => retention_expiry(database, &repository).await,
        Scenario::AuditRepair => audit_repair(database, &repository).await,
        Scenario::AuditDenied => audit_denied(database, &repository).await,
    }
}

async fn snapshot_boundary(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let valid = match repository
        .snapshot(OrganizationReadRequest {
            scope: scope(ORG_SNAPSHOT)?,
            actor_id: user(USER_SNAPSHOT_OWNER)?,
            identity_revision: revision(1)?,
            session_version: revision(0)?,
        })
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => return Ok(json!({"valid_error": error_code(error)})),
    };
    let cross = match repository
        .snapshot(OrganizationReadRequest {
            scope: scope(ORG_OWNER)?,
            actor_id: user(USER_SNAPSHOT_OWNER)?,
            identity_revision: revision(1)?,
            session_version: revision(0)?,
        })
        .await
    {
        Ok(_) => return Ok(json!({"cross_tenant": "unexpected_allow"})),
        Err(error) => error,
    };
    let unknown = match repository
        .snapshot(OrganizationReadRequest {
            scope: scope("018f47a6-7b1c-7f55-8f39-8f8a8690efff")?,
            actor_id: user(USER_SNAPSHOT_OWNER)?,
            identity_revision: revision(1)?,
            session_version: revision(0)?,
        })
        .await
    {
        Ok(_) => return Ok(json!({"unknown": "unexpected_allow"})),
        Err(error) => error,
    };
    Ok(json!({
        "role": role_code(valid.actor_membership.role),
        "status": "active",
        "cross_tenant": error_code(cross),
        "unknown": error_code(unknown),
        "indistinguishable": cross == unknown,
    }))
}

async fn invite_accept(
    repository: &D1OrganizationRepository<'_>,
    variant: u8,
    token_seed: u64,
) -> Result<Value> {
    let command = AcceptOrganizationInviteCommand {
        context: context(
            ORG_INVITE,
            USER_INVITEE,
            if variant == 1 {
                "018f47a6-7b1c-7f55-8f39-8f8a8690d141"
            } else {
                "018f47a6-7b1c-7f55-8f39-8f8a8690d142"
            },
            if variant == 1 {
                "invite-accept-a-0001"
            } else {
                "invite-accept-b-0001"
            },
            OrganizationAction::AcceptInvite,
            0,
            0,
            0,
            0,
            None,
        )?,
        invite_id: frame_domain::OrganizationInviteId::parse(INVITE)
            .map_err(|_| fixed_failure())?,
        presented_token_digest: digest(token_seed)?,
        expected_invite_revision: revision(0)?,
    };
    Ok(match repository.accept_invite(command).await {
        Ok(receipt) => json!({
            "result": receipt.result.stable_code(),
            "replayed": receipt.replayed,
        }),
        Err(error) => json!({"error": error_code(error)}),
    })
}

async fn invite_replay_and_mismatch(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let replay_a = invite_accept(repository, 1, 21).await?;
    let replay_b = invite_accept(repository, 2, 21).await?;
    let mismatch_a = invite_accept(repository, 1, 22).await?;
    let mismatch_b = invite_accept(repository, 2, 22).await?;
    Ok(json!({
        "replay_a": replay_a,
        "replay_b": replay_b,
        "mismatch_a": mismatch_a,
        "mismatch_b": mismatch_b,
    }))
}

async fn ownership_transfer(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let command = TransferOrganizationOwnershipCommand {
        context: context(
            ORG_OWNER,
            USER_OLD_OWNER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d151",
            "owner-transfer-0001",
            OrganizationAction::TransferOwnership,
            0,
            0,
            0,
            0,
            None,
        )?,
        new_owner_id: user(USER_NEW_OWNER)?,
        expected_new_owner_membership_revision: revision(0)?,
    };
    Ok(match repository.transfer_ownership(command).await {
        Ok(receipt) => json!({"result": receipt.result.stable_code()}),
        Err(error) => json!({"error": error_code(error)}),
    })
}

async fn owner_target_removal(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let command = ChangeOrganizationMemberCommand {
        context: context(
            ORG_OWNER,
            USER_OLD_OWNER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d152",
            "owner-remove-target-0001",
            OrganizationAction::RemoveMember,
            0,
            0,
            0,
            0,
            None,
        )?,
        subject_user_id: user(USER_NEW_OWNER)?,
        role: OrganizationRole::Member,
        state: MembershipState::Removed,
        has_pro_seat: false,
        expected_subject_revision: revision(0)?,
    };
    Ok(match repository.change_member(command).await {
        Ok(receipt) => json!({"result": receipt.result.stable_code()}),
        Err(error) => json!({"error": error_code(error)}),
    })
}

async fn ownership_invariant(database: &D1Database) -> Result<Value> {
    let row = database
        .prepare(
            "SELECT o.owner_id, \
             (SELECT COUNT(*) FROM organization_members m WHERE m.organization_id=o.id AND m.role='owner' AND m.state='active') AS active_owner_count, \
             (SELECT COUNT(*) FROM organization_members m WHERE m.organization_id=o.id AND m.user_id=o.owner_id AND m.role='owner' AND m.state='active') AS pointer_owner_count \
             FROM organizations o WHERE o.id=?1",
        )
        .bind(&[JsValue::from_str(ORG_OWNER)])?
        .first::<OwnershipInvariantRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if row.active_owner_count != 1 || row.pointer_owner_count != 1 {
        return Err(fixed_failure());
    }
    Ok(json!({
        "owner_id": row.owner_id,
        "active_owner_count": row.active_owner_count,
        "pointer_owner_count": row.pointer_owner_count,
    }))
}

async fn authority_member_downgrade(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let command = ChangeOrganizationMemberCommand {
        context: context(
            ORG_AUTHORITY_RACE,
            USER_AUTHORITY_OWNER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d181",
            "authority-downgrade-0001",
            OrganizationAction::ChangeMemberRole,
            0,
            0,
            0,
            0,
            None,
        )?,
        subject_user_id: user(USER_AUTHORITY_MEMBER)?,
        role: OrganizationRole::Viewer,
        state: MembershipState::Active,
        has_pro_seat: false,
        expected_subject_revision: revision(0)?,
    };
    Ok(match repository.change_member(command).await {
        Ok(receipt) => json!({"result": receipt.result.stable_code()}),
        Err(error) => json!({"error": error_code(error)}),
    })
}

async fn authority_folder_create(
    repository: &D1OrganizationRepository<'_>,
    folder_id: &str,
    after_downgrade: bool,
) -> Result<Value> {
    let command = CreateFolderCommand {
        context: context(
            ORG_AUTHORITY_RACE,
            USER_AUTHORITY_MEMBER,
            if after_downgrade {
                "018f47a6-7b1c-7f55-8f39-8f8a8690d183"
            } else {
                "018f47a6-7b1c-7f55-8f39-8f8a8690d182"
            },
            if after_downgrade {
                "authority-post-write-0001"
            } else {
                "authority-race-write-0001"
            },
            OrganizationAction::CreateFolder,
            0,
            0,
            0,
            0,
            Some(0),
        )?,
        folder: FolderRecord {
            id: folder(folder_id)?,
            scope: scope(ORG_AUTHORITY_RACE)?,
            space_id: space(SPACE_AUTHORITY_RACE)?,
            parent_id: None,
            created_by_user_id: user(USER_AUTHORITY_MEMBER)?,
            name: CollaborationName::parse(if after_downgrade {
                "Post downgrade"
            } else {
                "Race write"
            })
            .map_err(|_| fixed_failure())?,
            is_public: false,
            settings: OrganizationSettings::default(),
            depth: 0,
            created_at: time(NOW_MS)?,
            updated_at: time(NOW_MS)?,
            deleted_at: None,
            revision: OrganizationRevision::INITIAL,
            tree_revision: OrganizationRevision::INITIAL,
        },
        expected_parent_revision: None,
        expected_space_revision: revision(0)?,
    };
    Ok(match repository.create_folder(command).await {
        Ok(receipt) => json!({"result": receipt.result.stable_code()}),
        Err(error) => json!({"error": error_code(error)}),
    })
}

async fn authority_invariant(database: &D1Database) -> Result<Value> {
    let row = database
        .prepare(
            "SELECT m.role, m.state, i.session_version, \
             (SELECT COUNT(*) FROM folders f WHERE f.id=?3 AND f.organization_id=?1) AS race_folders, \
             (SELECT COUNT(*) FROM folders f WHERE f.id=?4 AND f.organization_id=?1) AS post_downgrade_folders \
             FROM organization_members m JOIN auth_identities_v2 i ON i.user_id=m.user_id \
             WHERE m.organization_id=?1 AND m.user_id=?2",
        )
        .bind(&[
            JsValue::from_str(ORG_AUTHORITY_RACE),
            JsValue::from_str(USER_AUTHORITY_MEMBER),
            JsValue::from_str(FOLDER_AUTHORITY_RACE),
            JsValue::from_str(FOLDER_AFTER_DOWNGRADE),
        ])?
        .first::<AuthorityInvariantRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if row.role != "viewer"
        || row.state != "active"
        || row.session_version != 1
        || !matches!(row.race_folders, 0 | 1)
        || row.post_downgrade_folders != 0
    {
        return Err(fixed_failure());
    }
    Ok(json!({
        "role": row.role,
        "state": row.state,
        "session_version": row.session_version,
        "race_folders": row.race_folders,
        "post_downgrade_folders": row.post_downgrade_folders,
    }))
}

async fn folder_cycle(repository: &D1OrganizationRepository<'_>) -> Result<Value> {
    let command = MoveFolderCommand {
        context: context(
            ORG_FOLDER,
            USER_FOLDER_MEMBER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d161",
            "folder-cycle-0001",
            OrganizationAction::MoveFolder,
            0,
            0,
            0,
            0,
            Some(0),
        )?,
        folder_id: folder(FOLDER_ROOT)?,
        space_id: space(SPACE_FOLDER)?,
        new_parent_id: Some(folder(FOLDER_GRANDCHILD)?),
        expected_folder_revision: revision(0)?,
        expected_space_revision: revision(0)?,
        expected_parent_revision: Some(revision(0)?),
        expected_tree_revision: revision(0)?,
    };
    let error = repository
        .move_folder(command)
        .await
        .expect_err("cycle must fail");
    Ok(json!({"cycle": error_code(error)}))
}

async fn folder_move(
    database: &D1Database,
    repository: &D1OrganizationRepository<'_>,
) -> Result<Value> {
    let command = MoveFolderCommand {
        context: context(
            ORG_FOLDER,
            USER_FOLDER_MEMBER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d162",
            "folder-move-0001",
            OrganizationAction::MoveFolder,
            0,
            0,
            0,
            0,
            Some(0),
        )?,
        folder_id: folder(FOLDER_CHILD)?,
        space_id: space(SPACE_FOLDER)?,
        new_parent_id: None,
        expected_folder_revision: revision(0)?,
        expected_space_revision: revision(0)?,
        expected_parent_revision: None,
        expected_tree_revision: revision(0)?,
    };
    let receipt = repository
        .move_folder(command)
        .await
        .map_err(|_| fixed_failure())?;
    let row = database
        .prepare(
            "SELECT f.parent_id, f.depth, \
             (SELECT COUNT(*) FROM organization_folder_closure_v1 c WHERE c.organization_id=f.organization_id AND c.space_id=f.space_id AND c.ancestor_id=f.id AND c.descendant_id=f.id AND c.distance=0) AS self_edges, \
             (SELECT COUNT(*) FROM organization_folder_closure_v1 c WHERE c.organization_id=f.organization_id AND c.space_id=f.space_id AND c.ancestor_id=c.descendant_id AND c.distance<>0) AS cycle_edges \
             FROM folders f WHERE f.id=?1 AND f.organization_id=?2",
        )
        .bind(&[
            JsValue::from_str(FOLDER_CHILD),
            JsValue::from_str(ORG_FOLDER),
        ])?
        .first::<FolderInvariantRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if row.parent_id.is_some() || row.depth != 0 || row.self_edges != 1 || row.cycle_edges != 0 {
        return Err(fixed_failure());
    }
    Ok(json!({
        "result": receipt.result.stable_code(),
        "depth": row.depth,
        "self_edges": row.self_edges,
        "cycle_edges": row.cycle_edges,
    }))
}

async fn tombstone_lifecycle(
    database: &D1Database,
    repository: &D1OrganizationRepository<'_>,
) -> Result<Value> {
    let tombstone = TombstoneOrganizationCommand {
        context: context(
            ORG_TOMBSTONE,
            USER_TOMBSTONE_OWNER,
            "018f47a6-7b1c-7f55-8f39-8f8a8690d171",
            "org-tombstone-0001",
            OrganizationAction::TombstoneOrganization,
            0,
            0,
            0,
            0,
            None,
        )?,
    };
    let tombstoned = repository
        .tombstone_organization(tombstone)
        .await
        .map_err(|_| fixed_failure())?;
    let tombstone_row = database
        .prepare("SELECT tombstoned_at_ms FROM organizations WHERE id=?1 AND status='tombstoned'")
        .bind(&[JsValue::from_str(ORG_TOMBSTONE)])?
        .first::<TombstoneRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    let stale = repository
        .tombstone_organization(TombstoneOrganizationCommand {
            context: context(
                ORG_TOMBSTONE,
                USER_TOMBSTONE_OWNER,
                "018f47a6-7b1c-7f55-8f39-8f8a8690d172",
                "org-tombstone-stale-0001",
                OrganizationAction::TombstoneOrganization,
                0,
                0,
                0,
                0,
                None,
            )?,
        })
        .await
        .expect_err("old fence must fail");
    let recovered = repository
        .recover_organization(RecoverOrganizationCommand {
            context: context(
                ORG_TOMBSTONE,
                USER_TOMBSTONE_OWNER,
                "018f47a6-7b1c-7f55-8f39-8f8a8690d173",
                "org-recover-0001",
                OrganizationAction::RecoverOrganization,
                1,
                1,
                0,
                0,
                None,
            )?,
            expected_tombstoned_at: time(tombstone_row.tombstoned_at_ms)?,
        })
        .await
        .map_err(|_| fixed_failure())?;
    Ok(json!({
        "tombstone": tombstoned.result.stable_code(),
        "stale_old_fence": error_code(stale),
        "recover": recovered.result.stable_code(),
    }))
}

async fn retention_expiry(
    database: &D1Database,
    repository: &D1OrganizationRepository<'_>,
) -> Result<Value> {
    let tombstone_row = database
        .prepare("SELECT tombstoned_at_ms FROM organizations WHERE id=?1 AND status='tombstoned'")
        .bind(&[JsValue::from_str(ORG_RETENTION)])?
        .first::<TombstoneRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    let recovery_context = context(
        ORG_RETENTION,
        USER_TOMBSTONE_OWNER,
        "018f47a6-7b1c-7f55-8f39-8f8a8690d192",
        "retention-recover-0001",
        OrganizationAction::RecoverOrganization,
        1,
        1,
        0,
        0,
        None,
    )?;
    let expired = repository
        .recover_organization(RecoverOrganizationCommand {
            context: recovery_context,
            expected_tombstoned_at: time(tombstone_row.tombstoned_at_ms)?,
        })
        .await
        .expect_err("expired recovery must fail");
    Ok(json!({
        "tombstone": "seeded_expired",
        "expired_recovery": error_code(expired),
    }))
}

async fn audit_repair(
    database: &D1Database,
    repository: &D1OrganizationRepository<'_>,
) -> Result<Value> {
    let request = OrganizationGraphAuditRequest {
        scope: scope(ORG_AUDIT)?,
        support_actor_id: user(USER_SUPPORT)?,
        support_ticket_digest: digest(91)?,
        identity_revision: revision(1)?,
        session_version: revision(0)?,
        occurred_at: time(NOW_MS)?,
        maximum_findings: 100,
    };
    let audit = match repository.audit_graph(request.clone()).await {
        Ok(audit) => audit,
        Err(error) => return Ok(json!({"audit_error": error_code(error)})),
    };
    if !audit
        .findings
        .iter()
        .any(|finding| finding.kind == OrganizationGraphFindingKind::MissingOwnerMembership)
    {
        return Err(fixed_failure());
    }
    let plan = match repository.plan_repair(request).await {
        Ok(plan) => plan,
        Err(error) => return Ok(json!({"plan_error": error_code(error)})),
    };
    let owner_memberships = database
        .prepare(
            "SELECT COUNT(*) AS count FROM organization_members WHERE organization_id=?1 AND role='owner' AND state='active'",
        )
        .bind(&[JsValue::from_str(ORG_AUDIT)])?
        .first::<CountRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if !plan.dry_run || owner_memberships.count != 0 {
        return Err(fixed_failure());
    }
    Ok(json!({
        "finding_count": audit.findings.len(),
        "plan_steps": plan.steps.len(),
        "dry_run": plan.dry_run,
        "automatic_mutations": 0,
    }))
}

async fn audit_denied(
    database: &D1Database,
    repository: &D1OrganizationRepository<'_>,
) -> Result<Value> {
    let before = database
        .prepare("SELECT COUNT(*) AS count FROM organization_audit_events_v1")
        .first::<CountRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    let denied = repository
        .audit_graph(OrganizationGraphAuditRequest {
            scope: scope(ORG_AUDIT)?,
            support_actor_id: user(USER_SUPPORT)?,
            support_ticket_digest: digest(92)?,
            identity_revision: revision(1)?,
            session_version: revision(0)?,
            occurred_at: time(NOW_MS)?,
            maximum_findings: 100,
        })
        .await
        .expect_err("invalid support ticket must fail");
    let after = database
        .prepare("SELECT COUNT(*) AS count FROM organization_audit_events_v1")
        .first::<CountRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if before.count != after.count {
        return Err(fixed_failure());
    }
    Ok(json!({
        "error": error_code(denied),
        "tenant_audit_rows_added": 0,
    }))
}

#[derive(Debug, Deserialize)]
struct CountRow {
    count: i64,
}

#[allow(clippy::too_many_arguments)]
fn context(
    organization_id: &str,
    actor_id: &str,
    operation_id: &str,
    idempotency_key: &str,
    action: OrganizationAction,
    organization_revision: u64,
    organization_authority_version: u64,
    membership_revision: u64,
    membership_authority_version: u64,
    space_membership_revision: Option<u64>,
) -> Result<OrganizationMutationContext> {
    Ok(OrganizationMutationContext {
        authority: OrganizationWriteAuthority {
            operation_id: OrganizationOperationId::parse(operation_id)
                .map_err(|_| fixed_failure())?,
            scope: scope(organization_id)?,
            actor_id: user(actor_id)?,
            fence: OrganizationAuthorityFence {
                identity_revision: revision(1)?,
                session_version: revision(0)?,
                organization_revision: revision(organization_revision)?,
                organization_authority_version: revision(organization_authority_version)?,
                membership_revision: revision(membership_revision)?,
                membership_authority_version: revision(membership_authority_version)?,
                space_membership_revision: space_membership_revision.map(revision).transpose()?,
            },
            occurred_at: time(NOW_MS)?,
        },
        idempotency_key: IdempotencyKey::parse(idempotency_key).map_err(|_| fixed_failure())?,
        action,
    })
}

fn scope(value: &str) -> Result<OrganizationScope> {
    OrganizationScope::from_organization(OrganizationId::parse(value).map_err(|_| fixed_failure())?)
        .map_err(|_| fixed_failure())
}

fn user(value: &str) -> Result<UserId> {
    UserId::parse(value).map_err(|_| fixed_failure())
}

fn folder(value: &str) -> Result<FolderId> {
    FolderId::parse(value).map_err(|_| fixed_failure())
}

fn space(value: &str) -> Result<SpaceId> {
    SpaceId::parse(value).map_err(|_| fixed_failure())
}

fn revision(value: u64) -> Result<OrganizationRevision> {
    OrganizationRevision::new(value).map_err(|_| fixed_failure())
}

fn digest(value: u64) -> Result<SecretDigest> {
    SecretDigest::parse_sha256(format!("{value:064x}")).map_err(|_| fixed_failure())
}

fn time(value: i64) -> Result<TimestampMillis> {
    TimestampMillis::new(value).map_err(|_| fixed_failure())
}

const fn role_code(role: OrganizationRole) -> &'static str {
    match role {
        OrganizationRole::Owner => "owner",
        OrganizationRole::Admin => "admin",
        OrganizationRole::Member => "member",
        OrganizationRole::Viewer => "viewer",
    }
}

const fn error_code(error: OrganizationPortError) -> &'static str {
    match error {
        OrganizationPortError::AccessDenied => "access_denied",
        OrganizationPortError::StaleAuthority => "stale_authority",
        OrganizationPortError::Conflict => "conflict",
        OrganizationPortError::Invalid => "invalid",
        OrganizationPortError::RetentionLocked => "retention_locked",
        OrganizationPortError::Unavailable => "unavailable",
        OrganizationPortError::Corrupt => "corrupt",
    }
}

fn valid_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn fixed_failure() -> worker::Error {
    worker::Error::RustError("organization repository conformance failed".into())
}

fn fixed_response(status: u16, outcome: &'static str, details: Option<Value>) -> Result<Response> {
    let mut body = json!({"outcome": outcome});
    if let (Some(object), Some(details)) = (body.as_object_mut(), details) {
        object.insert("details".into(), details);
    }
    Response::from_json(&body).map(|response| response.with_status(status))
}
