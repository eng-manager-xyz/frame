//! Atomic D1 adapter for the source-pinned invite accept/decline routes.

use async_trait::async_trait;
use frame_application::{
    LegacyInviteActionV1, LegacyInviteAtomicPortV1, LegacyInviteCommandV1, LegacyInviteErrorV1,
    LegacyInviteReceiptV1,
};
use frame_domain::{LegacyCapNanoId, OrganizationId};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const SNAPSHOT_SQL: &str = include_str!("../queries/legacy_invite_lifecycle/snapshot.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/operation_insert.sql");
const AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/authority_assert.sql");
const ACCEPT_MEMBERSHIP_INSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/accept_membership_insert.sql");
const ACCEPT_MEMBER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/accept_member_alias_insert.sql");
const ACCEPT_PRO_SEAT_UPDATE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/accept_pro_seat_update.sql");
const ACCEPT_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/accept_user_update.sql");
const DECLINE_SPACE_MEMBERS_DELETE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/decline_space_members_delete.sql");
const DECLINE_MEMBER_ALIAS_UPDATE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/decline_member_alias_update.sql");
const DECLINE_MEMBERSHIP_DELETE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/decline_membership_delete.sql");
const DECLINE_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/decline_user_update.sql");
const INVITE_DELETE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/invite_delete.sql");
const INVITE_ALIAS_RESOLVE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/invite_alias_resolve.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/receipt_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_invite_lifecycle/audit_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/operation_complete.sql");
const POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/postcondition_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_invite_lifecycle/assertion_cleanup.sql");

type InviteResult<T> = Result<T, LegacyInviteErrorV1>;

#[derive(Debug, Deserialize)]
struct InviteSnapshotRowV1 {
    actor_email: String,
    mapped_invite_id: String,
    organization_id: String,
    invited_email: String,
    legacy_role: String,
    owner_id: String,
    owner_invite_quota: Option<i64>,
    owner_subscription_id: Option<String>,
    membership_exists: i64,
    membership_has_pro_seat: i64,
    mapped_member_id: Option<String>,
    legacy_member_id: Option<String>,
    pro_seats_used: i64,
    fallback_organization_id: Option<String>,
    other_pro_seat_count: i64,
}

impl InviteSnapshotRowV1 {
    fn validate(&self) -> InviteResult<()> {
        if Uuid::parse_str(&self.mapped_invite_id).is_err()
            || OrganizationId::parse(&self.organization_id).is_err()
            || frame_domain::UserId::parse(&self.owner_id).is_err()
            || self.actor_email.is_empty()
            || self.actor_email.len() > 255
            || self.invited_email.is_empty()
            || self.invited_email.len() > 255
            || self.legacy_role.is_empty()
            || self.legacy_role.len() > 255
            || !matches!(self.membership_exists, 0 | 1)
            || !matches!(self.membership_has_pro_seat, 0 | 1)
            || self.pro_seats_used < 0
            || self.other_pro_seat_count < 0
            || self.owner_invite_quota.is_some_and(|value| value < 0)
            || self
                .fallback_organization_id
                .as_deref()
                .is_some_and(|value| OrganizationId::parse(value).is_err())
            || (self.membership_exists == 1
                && (self
                    .mapped_member_id
                    .as_deref()
                    .is_none_or(|value| Uuid::parse_str(value).is_err())
                    || self
                        .legacy_member_id
                        .as_deref()
                        .is_none_or(|value| LegacyCapNanoId::parse(value).is_err())))
            || (self.membership_exists == 0
                && (self.mapped_member_id.is_some() || self.legacy_member_id.is_some()))
        {
            return Err(LegacyInviteErrorV1::Internal);
        }
        Ok(())
    }
}

pub(crate) struct D1LegacyInviteAtomicPortV1<'database> {
    database: &'database D1Database,
    now_ms: i64,
    cap_hosted: bool,
}

impl<'database> D1LegacyInviteAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(
        database: &'database D1Database,
        now_ms: i64,
        cap_hosted: bool,
    ) -> Self {
        Self {
            database,
            now_ms,
            cap_hosted,
        }
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> InviteResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyInviteErrorV1::Internal)
    }

    async fn snapshot(&self, command: &LegacyInviteCommandV1) -> InviteResult<InviteSnapshotRowV1> {
        let result = self
            .statement(
                SNAPSHOT_SQL,
                &[
                    JsValue::from_str(&command.actor_id().to_string()),
                    JsValue::from_str(command.legacy_invite_id()),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyInviteErrorV1::Internal)?;
        if !result.success() {
            return Err(LegacyInviteErrorV1::Internal);
        }
        let mut rows = result
            .results::<InviteSnapshotRowV1>()
            .map_err(|_| LegacyInviteErrorV1::Internal)?;
        if rows.len() > 1 {
            return Err(LegacyInviteErrorV1::Internal);
        }
        let row = rows.pop().ok_or(LegacyInviteErrorV1::InviteNotFound)?;
        row.validate()?;
        if row.actor_email.to_lowercase() != row.invited_email.to_lowercase() {
            return Err(LegacyInviteErrorV1::EmailMismatch);
        }
        Ok(row)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> InviteResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyInviteErrorV1::Internal)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyInviteErrorV1::Internal);
        }
        Ok(())
    }

    fn common_prefix(
        &self,
        command: &LegacyInviteCommandV1,
        snapshot: &InviteSnapshotRowV1,
        operation_id: &str,
        action: &str,
    ) -> InviteResult<Vec<D1PreparedStatement>> {
        let actor_id = command.actor_id().to_string();
        let owner_invite_quota = snapshot
            .owner_invite_quota
            .map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64));
        let owner_subscription_id = snapshot
            .owner_subscription_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        let mapped_member_id = snapshot
            .mapped_member_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        let legacy_member_id = snapshot
            .legacy_member_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        let fallback_organization_id = snapshot
            .fallback_organization_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        Ok(vec![
            self.statement(
                OPERATION_INSERT_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(command.legacy_invite_id()),
                    JsValue::from_str(action),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?,
            self.statement(
                AUTHORITY_ASSERT_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(command.legacy_invite_id()),
                    JsValue::from_str(&snapshot.mapped_invite_id),
                    JsValue::from_str(&snapshot.invited_email),
                    JsValue::from_str(&snapshot.legacy_role),
                    JsValue::from_str(&snapshot.actor_email),
                    JsValue::from_str(&snapshot.owner_id),
                    owner_invite_quota,
                    owner_subscription_id,
                    JsValue::from_f64(snapshot.membership_exists as f64),
                    JsValue::from_f64(snapshot.membership_has_pro_seat as f64),
                    mapped_member_id,
                    legacy_member_id,
                    JsValue::from_f64(snapshot.pro_seats_used as f64),
                    fallback_organization_id,
                    JsValue::from_f64(snapshot.other_pro_seat_count as f64),
                ],
            )?,
        ])
    }

    #[allow(clippy::too_many_arguments)]
    fn common_suffix(
        &self,
        command: &LegacyInviteCommandV1,
        snapshot: &InviteSnapshotRowV1,
        operation_id: &str,
        action: &str,
        decision: &str,
        membership_created: bool,
        membership_removed: bool,
        pro_seat_assigned: bool,
        inherited_subscription_cleared: bool,
    ) -> InviteResult<Vec<D1PreparedStatement>> {
        let actor_id = command.actor_id().to_string();
        let fallback = snapshot
            .fallback_organization_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        Ok(vec![
            self.statement(
                INVITE_DELETE_SQL,
                &[
                    JsValue::from_str(&snapshot.mapped_invite_id),
                    JsValue::from_str(&snapshot.organization_id),
                ],
            )?,
            self.statement(
                INVITE_ALIAS_RESOLVE_SQL,
                &[
                    JsValue::from_str(&snapshot.mapped_invite_id),
                    JsValue::from_str(decision),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_str(operation_id),
                ],
            )?,
            self.statement(
                RECEIPT_INSERT_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(action),
                    JsValue::from_f64(snapshot.membership_exists as f64),
                    JsValue::from_bool(membership_created),
                    JsValue::from_bool(membership_removed),
                    JsValue::from_bool(pro_seat_assigned),
                    JsValue::from_bool(inherited_subscription_cleared),
                    fallback,
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(action),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?,
            self.statement(
                POSTCONDITION_ASSERT_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&snapshot.mapped_invite_id),
                    JsValue::from_str(decision),
                    JsValue::from_str(action),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(operation_id)])?,
        ])
    }

    async fn accept(
        &self,
        command: &LegacyInviteCommandV1,
        snapshot: &InviteSnapshotRowV1,
        operation_id: &str,
    ) -> InviteResult<LegacyInviteReceiptV1> {
        let actor_id = command.actor_id().to_string();
        let membership_created = snapshot.membership_exists == 0;
        let owner_subscription = snapshot
            .owner_subscription_id
            .as_deref()
            .filter(|value| !value.is_empty());
        let remaining = snapshot
            .owner_invite_quota
            .unwrap_or(1)
            .saturating_sub(snapshot.pro_seats_used);
        let pro_seat_assigned = membership_created
            && owner_subscription.is_some()
            && (!self.cap_hosted || remaining > 0);
        let role = if snapshot.legacy_role.eq_ignore_ascii_case("admin") {
            "admin"
        } else {
            "member"
        };
        let generated_legacy_member_id = derived_member_nanoid(operation_id);
        let generated_mapped_member_id = LegacyCapNanoId::parse(&generated_legacy_member_id)
            .map_err(|_| LegacyInviteErrorV1::Internal)?
            .mapped_uuid()
            .to_string();
        let mut statements = self.common_prefix(command, snapshot, operation_id, "accept")?;
        statements.extend([
            self.statement(
                ACCEPT_MEMBERSHIP_INSERT_SQL,
                &[
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(role),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(snapshot.membership_exists as f64),
                ],
            )?,
            self.statement(
                ACCEPT_MEMBER_ALIAS_INSERT_SQL,
                &[
                    JsValue::from_str(&generated_mapped_member_id),
                    JsValue::from_str(&generated_legacy_member_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(snapshot.membership_exists as f64),
                ],
            )?,
            self.statement(
                ACCEPT_PRO_SEAT_UPDATE_SQL,
                &[
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_bool(pro_seat_assigned),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_str(operation_id),
                ],
            )?,
            self.statement(
                ACCEPT_USER_UPDATE_SQL,
                &[
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_bool(pro_seat_assigned),
                    owner_subscription.map_or(JsValue::NULL, JsValue::from_str),
                    JsValue::from_f64(self.now_ms as f64),
                ],
            )?,
        ]);
        statements.extend(self.common_suffix(
            command,
            snapshot,
            operation_id,
            "accept",
            "accepted",
            membership_created,
            false,
            pro_seat_assigned,
            false,
        )?);
        self.batch(statements).await?;
        Ok(LegacyInviteReceiptV1 {
            action: LegacyInviteActionV1::Accept,
            organization_id: OrganizationId::parse(&snapshot.organization_id)
                .map_err(|_| LegacyInviteErrorV1::Internal)?,
            membership_created,
            membership_removed: false,
            pro_seat_assigned,
            inherited_subscription_cleared: false,
        })
    }

    async fn decline(
        &self,
        command: &LegacyInviteCommandV1,
        snapshot: &InviteSnapshotRowV1,
        operation_id: &str,
    ) -> InviteResult<LegacyInviteReceiptV1> {
        let actor_id = command.actor_id().to_string();
        let membership_removed = snapshot.membership_exists == 1;
        let inherited_subscription_cleared = membership_removed
            && snapshot.membership_has_pro_seat == 1
            && snapshot.other_pro_seat_count == 0;
        let fallback = snapshot
            .fallback_organization_id
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str);
        let mut statements = self.common_prefix(command, snapshot, operation_id, "decline")?;
        statements.extend([
            self.statement(
                DECLINE_SPACE_MEMBERS_DELETE_SQL,
                &[
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_bool(membership_removed),
                ],
            )?,
            self.statement(
                DECLINE_MEMBER_ALIAS_UPDATE_SQL,
                &[
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_str(operation_id),
                    JsValue::from_bool(membership_removed),
                ],
            )?,
            self.statement(
                DECLINE_MEMBERSHIP_DELETE_SQL,
                &[
                    JsValue::from_str(&snapshot.organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_bool(membership_removed),
                ],
            )?,
            self.statement(
                DECLINE_USER_UPDATE_SQL,
                &[
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&snapshot.organization_id),
                    fallback,
                    JsValue::from_bool(inherited_subscription_cleared),
                    JsValue::from_f64(self.now_ms as f64),
                    JsValue::from_bool(membership_removed),
                ],
            )?,
        ]);
        statements.extend(self.common_suffix(
            command,
            snapshot,
            operation_id,
            "decline",
            "declined",
            false,
            membership_removed,
            false,
            inherited_subscription_cleared,
        )?);
        self.batch(statements).await?;
        Ok(LegacyInviteReceiptV1 {
            action: LegacyInviteActionV1::Decline,
            organization_id: OrganizationId::parse(&snapshot.organization_id)
                .map_err(|_| LegacyInviteErrorV1::Internal)?,
            membership_created: false,
            membership_removed,
            pro_seat_assigned: false,
            inherited_subscription_cleared,
        })
    }
}

#[async_trait]
impl LegacyInviteAtomicPortV1 for D1LegacyInviteAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyInviteCommandV1,
    ) -> InviteResult<LegacyInviteReceiptV1> {
        if !(0..=9_007_199_254_740_991).contains(&self.now_ms) {
            return Err(LegacyInviteErrorV1::Internal);
        }
        let snapshot = self.snapshot(command).await?;
        let operation_id = Uuid::now_v7().to_string();
        match command.action() {
            LegacyInviteActionV1::Accept => self.accept(command, &snapshot, &operation_id).await,
            LegacyInviteActionV1::Decline => self.decline(command, &snapshot, &operation_id).await,
        }
    }
}

fn derived_member_nanoid(operation_id: &str) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let digest = Sha256::digest(format!("frame-invite-member-v1\0{operation_id}").as_bytes());
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut output = String::with_capacity(15);
    for byte in digest {
        accumulator = (accumulator << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 && output.len() < 15 {
            bits -= 5;
            output.push(ALPHABET[((accumulator >> bits) & 31) as usize] as char);
        }
        if output.len() == 15 {
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_member_alias_is_valid_and_stable() {
        let operation = "00000000-0000-4000-8000-000000000001";
        let first = derived_member_nanoid(operation);
        assert_eq!(first, derived_member_nanoid(operation));
        assert!(LegacyCapNanoId::parse(first).is_ok());
    }

    #[test]
    fn checked_in_queries_cover_every_source_side_effect() {
        for (query, needle) in [
            (ACCEPT_USER_UPDATE_SQL, "organizationSetup"),
            (ACCEPT_USER_UPDATE_SQL, "customDomain"),
            (ACCEPT_USER_UPDATE_SQL, "inviteTeam"),
            (ACCEPT_PRO_SEAT_UPDATE_SQL, "has_pro_seat"),
            (DECLINE_SPACE_MEMBERS_DELETE_SQL, "space_members"),
            (DECLINE_USER_UPDATE_SQL, "third_party_stripe"),
            (INVITE_DELETE_SQL, "organization_invites"),
            (POSTCONDITION_ASSERT_SQL, "durable_exact_postcondition"),
        ] {
            assert!(query.contains(needle), "missing {needle}");
        }
    }
}
