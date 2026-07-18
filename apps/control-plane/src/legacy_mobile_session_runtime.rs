//! Checked D1 and deterministic provider-effect orchestration for Cap mobile sessions.

use frame_application::{LEGACY_MOBILE_EMAIL_CODE_TTL_MS, legacy_mobile_provisioned_user_name};
use frame_domain::{LegacyCapNanoId, SealedDeliveryEnvelope};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const EMAIL_USER_EXISTS_SQL: &str =
    include_str!("../queries/legacy_mobile_session/email_user_exists.sql");
const HANDOFF_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/handoff_insert.sql");
const CHALLENGE_UPSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/challenge_upsert.sql");
const CHALLENGE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/challenge_snapshot.sql");
const CHALLENGE_DELETE_IDENTIFIER_SQL: &str =
    include_str!("../queries/legacy_mobile_session/challenge_delete_identifier.sql");
const CHALLENGE_DELETE_MATCHING_SQL: &str =
    include_str!("../queries/legacy_mobile_session/challenge_delete_matching.sql");
const USER_SNAPSHOT_SQL: &str = include_str!("../queries/legacy_mobile_session/user_snapshot.sql");
const PENDING_INVITE_SQL: &str =
    include_str!("../queries/legacy_mobile_session/pending_invite.sql");
const ACTOR_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/actor_authority_assert.sql");
const USER_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/user_authority_assert.sql");
const USER_VERIFY_VISIBLE_SQL: &str =
    include_str!("../queries/legacy_mobile_session/user_verify_visible.sql");
const USER_VERIFY_PROVISIONED_SQL: &str =
    include_str!("../queries/legacy_mobile_session/user_verify_provisioned.sql");
const USER_INSERT_SQL: &str = include_str!("../queries/legacy_mobile_session/user_insert.sql");
const USER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/user_alias_insert.sql");
const ORGANIZATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/organization_insert.sql");
const ORGANIZATION_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/organization_alias_insert.sql");
const MEMBER_INSERT_SQL: &str = include_str!("../queries/legacy_mobile_session/member_insert.sql");
const MEMBER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/member_alias_insert.sql");
const USER_ORGANIZATION_SELECT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/user_organization_select.sql");
const MOBILE_KEY_COUNT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_key_count.sql");
const MOBILE_KEYS_DELETE_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_keys_delete.sql");
const MOBILE_KEY_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_key_insert.sql");
const MOBILE_KEY_ACTOR_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_key_actor.sql");
const SESSION_ACTOR_SQL: &str = include_str!("../queries/legacy_mobile_session/session_actor.sql");
const MOBILE_KEY_REVOKE_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_key_revoke.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/operation_insert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/receipt_insert.sql");
const STRIPE_EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/stripe_effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_mobile_session/audit_insert.sql");
const POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/postcondition_assert.sql");
const CHALLENGE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/challenge_postcondition_assert.sql");
const MOBILE_KEY_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/mobile_key_postcondition_assert.sql");
const STRIPE_EFFECT_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/stripe_effect_postcondition_assert.sql");
const REVOKE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_session/revoke_postcondition_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_mobile_session/assertion_cleanup.sql");

type MobileResult<T> = std::result::Result<T, LegacyMobileSessionRuntimeFailureV1>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMobileSessionRuntimeFailureV1 {
    Forbidden,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyMobileSessionActorV1 {
    pub mapped_user_id: String,
    pub legacy_user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyMobileEmailVerifyOutcomeV1 {
    ApiKey {
        api_key: String,
        actor: LegacyMobileSessionActorV1,
    },
    StripeEffectPending {
        actor: LegacyMobileSessionActorV1,
    },
}

#[derive(Debug, Deserialize)]
struct ExistsRowV1 {
    user_exists: i64,
}

#[derive(Debug, Deserialize)]
struct ChallengeRowV1 {
    token_digest: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct UserSnapshotRowV1 {
    mapped_user_id: String,
    legacy_user_id: Option<String>,
    display_name: Option<String>,
    has_linked_account: i64,
    has_pending_provisioned_invite: i64,
}

#[derive(Debug, Deserialize)]
struct PendingInviteRowV1 {
    pending_invite: i64,
}

#[derive(Debug, Deserialize)]
struct CountRowV1 {
    key_count: i64,
}

#[derive(Debug, Deserialize)]
struct ActorRowV1 {
    mapped_user_id: String,
    legacy_user_id: Option<String>,
}

enum UserPlanV1 {
    ExistingVisible {
        actor: LegacyMobileSessionActorV1,
    },
    ExistingProvisioned {
        actor: LegacyMobileSessionActorV1,
    },
    New {
        actor: LegacyMobileSessionActorV1,
        pending_invite: bool,
        organization: Option<(LegacyCapNanoId, LegacyCapNanoId)>,
    },
}

pub(crate) struct D1LegacyMobileSessionV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyMobileSessionV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn email_user_exists(&self, email: &str) -> MobileResult<bool> {
        let row = self
            .first::<ExistsRowV1>(EMAIL_USER_EXISTS_SQL, &[JsValue::from_str(email)])
            .await?
            .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
        match row.user_exists {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LegacyMobileSessionRuntimeFailureV1::Corrupt),
        }
    }

    pub(crate) async fn request_email(
        &self,
        identifier_digest: &str,
        token_digest: &str,
        envelope: &SealedDeliveryEnvelope,
        now_ms: i64,
    ) -> MobileResult<()> {
        if !valid_digest(identifier_digest)
            || !valid_digest(token_digest)
            || envelope.sealed_payload().len() != 1_071
        {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        let operation_id = uuid::Uuid::now_v7().to_string();
        let audit_id = uuid::Uuid::now_v7().to_string();
        let delivery_id = envelope.id.to_string();
        let payload_hex = bytes_to_hex(envelope.sealed_payload());
        let payload_digest = sha256_hex(envelope.sealed_payload());
        let statements = vec![
            self.statement(
                HANDOFF_INSERT_SQL,
                &[
                    JsValue::from_str(&delivery_id),
                    JsValue::from_str(&payload_hex),
                    JsValue::from_str(&payload_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                CHALLENGE_UPSERT_SQL,
                &[
                    JsValue::from_str(identifier_digest),
                    JsValue::from_str(token_digest),
                    JsValue::from_str(&delivery_id),
                    number(now_ms),
                    JsValue::from_str(&operation_id),
                ],
            )?,
            self.operation(
                &operation_id,
                "email_request",
                None,
                identifier_digest,
                "email_handoff_pending",
                now_ms,
            )?,
            self.receipt(
                &operation_id,
                "challenge_replaced",
                None,
                None,
                None,
                Some(&delivery_id),
                0,
                now_ms,
            )?,
            self.audit(
                &audit_id,
                &operation_id,
                None,
                "email_request",
                identifier_digest,
                now_ms,
            )?,
            self.postcondition(
                &operation_id,
                "operation_receipt_audit",
                "email_request",
                "challenge_replaced",
            )?,
            self.statement(
                CHALLENGE_POSTCONDITION_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(identifier_digest),
                    JsValue::from_str(token_digest),
                    JsValue::from_str(&delivery_id),
                    number(now_ms),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&operation_id)])?,
        ];
        self.batch(statements).await
    }

    /// Destructive one-use verification: missing, expired, wrong, and raced
    /// tokens all return Forbidden. Expired and wrong attempts delete the live
    /// identifier exactly as Cap does.
    pub(crate) async fn consume_email_challenge(
        &self,
        identifier_digest: &str,
        token_digest: &str,
        now_ms: i64,
    ) -> MobileResult<()> {
        let Some(row) = self
            .first::<ChallengeRowV1>(
                CHALLENGE_SNAPSHOT_SQL,
                &[JsValue::from_str(identifier_digest)],
            )
            .await?
        else {
            return Err(LegacyMobileSessionRuntimeFailureV1::Forbidden);
        };
        if !valid_digest(&row.token_digest)
            || row.created_at_ms < 0
            || row.expires_at_ms != row.created_at_ms + LEGACY_MOBILE_EMAIL_CODE_TTL_MS
        {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        if row.expires_at_ms < now_ms || row.token_digest != token_digest {
            self.run(
                CHALLENGE_DELETE_IDENTIFIER_SQL,
                &[JsValue::from_str(identifier_digest)],
            )
            .await?;
            return Err(LegacyMobileSessionRuntimeFailureV1::Forbidden);
        }
        let result = self
            .run_result(
                CHALLENGE_DELETE_MATCHING_SQL,
                &[
                    JsValue::from_str(identifier_digest),
                    JsValue::from_str(token_digest),
                ],
            )
            .await?;
        let changes = result
            .meta()
            .ok()
            .flatten()
            .and_then(|meta| meta.changes)
            .ok_or(LegacyMobileSessionRuntimeFailureV1::Unavailable)?;
        if changes == 1 {
            Ok(())
        } else {
            Err(LegacyMobileSessionRuntimeFailureV1::Forbidden)
        }
    }

    pub(crate) async fn verify_email_user(
        &self,
        email: &str,
        email_digest: &str,
        stripe_available: bool,
        now_ms: i64,
    ) -> MobileResult<LegacyMobileEmailVerifyOutcomeV1> {
        let plan = self.user_plan(email).await?;
        let operation_id = uuid::Uuid::now_v7().to_string();
        let audit_id = uuid::Uuid::now_v7().to_string();
        let mut statements = Vec::new();
        let (actor, create_user_path) = match &plan {
            UserPlanV1::ExistingVisible { actor } => {
                statements.push(self.statement(
                    USER_AUTHORITY_ASSERT_SQL,
                    &[
                        JsValue::from_str(&operation_id),
                        JsValue::from_str(&actor.mapped_user_id),
                        JsValue::from_str(email),
                        JsValue::from_str(&actor.legacy_user_id),
                    ],
                )?);
                statements.push(self.statement(
                    USER_VERIFY_VISIBLE_SQL,
                    &[JsValue::from_str(&actor.mapped_user_id), number(now_ms)],
                )?);
                (actor.clone(), false)
            }
            UserPlanV1::ExistingProvisioned { actor } => {
                statements.push(self.statement(
                    USER_AUTHORITY_ASSERT_SQL,
                    &[
                        JsValue::from_str(&operation_id),
                        JsValue::from_str(&actor.mapped_user_id),
                        JsValue::from_str(email),
                        JsValue::from_str(&actor.legacy_user_id),
                    ],
                )?);
                statements.push(self.statement(
                    USER_VERIFY_PROVISIONED_SQL,
                    &[
                        JsValue::from_str(&actor.mapped_user_id),
                        JsValue::from_str(&legacy_mobile_provisioned_user_name(email)),
                        number(now_ms),
                    ],
                )?);
                (actor.clone(), true)
            }
            UserPlanV1::New {
                actor,
                pending_invite,
                organization,
            } => {
                statements.push(self.statement(
                    USER_INSERT_SQL,
                    &[
                        JsValue::from_str(&actor.mapped_user_id),
                        JsValue::from_str(email),
                        number(now_ms),
                    ],
                )?);
                statements.push(self.statement(
                    USER_ALIAS_INSERT_SQL,
                    &[
                        JsValue::from_str(&actor.legacy_user_id),
                        JsValue::from_str(&actor.mapped_user_id),
                        number(now_ms),
                    ],
                )?);
                if !pending_invite {
                    let (organization_id, member_id) = organization
                        .as_ref()
                        .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
                    let mapped_organization = organization_id.mapped_uuid().to_string();
                    let mapped_member = member_id.mapped_uuid().to_string();
                    statements.push(self.statement(
                        ORGANIZATION_INSERT_SQL,
                        &[
                            JsValue::from_str(&mapped_organization),
                            JsValue::from_str(&actor.mapped_user_id),
                            number(now_ms),
                        ],
                    )?);
                    statements.push(self.statement(
                        ORGANIZATION_ALIAS_INSERT_SQL,
                        &[
                            JsValue::from_str(&mapped_organization),
                            JsValue::from_str(organization_id.as_str()),
                            number(now_ms),
                            JsValue::from_str(&operation_id),
                        ],
                    )?);
                    statements.push(self.statement(
                        MEMBER_INSERT_SQL,
                        &[
                            JsValue::from_str(&mapped_organization),
                            JsValue::from_str(&actor.mapped_user_id),
                            number(now_ms),
                        ],
                    )?);
                    statements.push(self.statement(
                        MEMBER_ALIAS_INSERT_SQL,
                        &[
                            JsValue::from_str(&mapped_member),
                            JsValue::from_str(member_id.as_str()),
                            JsValue::from_str(&mapped_organization),
                            JsValue::from_str(&actor.mapped_user_id),
                            number(now_ms),
                            JsValue::from_str(&operation_id),
                        ],
                    )?);
                    statements.push(self.statement(
                        USER_ORGANIZATION_SELECT_SQL,
                        &[
                            JsValue::from_str(&actor.mapped_user_id),
                            JsValue::from_str(&mapped_organization),
                            JsValue::from_str(&operation_id),
                            number(now_ms),
                        ],
                    )?);
                }
                (actor.clone(), true)
            }
        };

        if create_user_path && stripe_available {
            statements.push(self.operation(
                &operation_id,
                "email_verify",
                Some(&actor.mapped_user_id),
                email_digest,
                "stripe_sync_pending",
                now_ms,
            )?);
            statements.push(self.statement(
                STRIPE_EFFECT_INSERT_SQL,
                &[
                    JsValue::from_str(&uuid::Uuid::now_v7().to_string()),
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(email_digest),
                    number(now_ms),
                ],
            )?);
            statements.push(self.receipt(
                &operation_id,
                "user_provisioned_provider_pending",
                Some(&actor.mapped_user_id),
                Some(&actor.legacy_user_id),
                None,
                None,
                0,
                now_ms,
            )?);
            statements.push(self.audit(
                &audit_id,
                &operation_id,
                Some(&actor.mapped_user_id),
                "email_verify",
                email_digest,
                now_ms,
            )?);
            statements.push(self.postcondition(
                &operation_id,
                "operation_receipt_audit",
                "email_verify",
                "user_provisioned_provider_pending",
            )?);
            statements.push(self.statement(
                STRIPE_EFFECT_POSTCONDITION_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(email_digest),
                ],
            )?);
            statements
                .push(self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&operation_id)])?);
            self.batch(statements).await?;
            return Ok(LegacyMobileEmailVerifyOutcomeV1::StripeEffectPending { actor });
        }

        let (api_key, key_row_id, key_digest, prior_count) =
            self.key_material(&actor.mapped_user_id).await?;
        statements.push(self.statement(
            MOBILE_KEYS_DELETE_SQL,
            &[JsValue::from_str(&actor.mapped_user_id)],
        )?);
        statements.push(self.statement(
            MOBILE_KEY_INSERT_SQL,
            &[
                JsValue::from_str(&key_row_id),
                JsValue::from_str(&actor.mapped_user_id),
                JsValue::from_str(&key_digest),
                number(now_ms),
            ],
        )?);
        statements.push(self.operation(
            &operation_id,
            "email_verify",
            Some(&actor.mapped_user_id),
            email_digest,
            "not_requested",
            now_ms,
        )?);
        statements.push(self.receipt(
            &operation_id,
            "api_key_replaced",
            Some(&actor.mapped_user_id),
            Some(&actor.legacy_user_id),
            Some(&key_row_id),
            None,
            prior_count.saturating_add(1),
            now_ms,
        )?);
        statements.push(self.audit(
            &audit_id,
            &operation_id,
            Some(&actor.mapped_user_id),
            "email_verify",
            email_digest,
            now_ms,
        )?);
        statements.push(self.postcondition(
            &operation_id,
            "operation_receipt_audit",
            "email_verify",
            "api_key_replaced",
        )?);
        statements.push(self.statement(
            MOBILE_KEY_POSTCONDITION_ASSERT_SQL,
            &[
                JsValue::from_str(&operation_id),
                JsValue::from_str(&key_row_id),
                JsValue::from_str(&actor.mapped_user_id),
                JsValue::from_str(&key_digest),
            ],
        )?);
        statements
            .push(self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&operation_id)])?);
        self.batch(statements).await?;
        Ok(LegacyMobileEmailVerifyOutcomeV1::ApiKey { api_key, actor })
    }

    pub(crate) async fn session_actor(
        &self,
        mapped_user_id: &str,
    ) -> MobileResult<Option<LegacyMobileSessionActorV1>> {
        self.actor(SESSION_ACTOR_SQL, &[JsValue::from_str(mapped_user_id)])
            .await
    }

    pub(crate) async fn api_key_actor(
        &self,
        api_key: &str,
        now_ms: i64,
    ) -> MobileResult<Option<LegacyMobileSessionActorV1>> {
        self.actor(
            MOBILE_KEY_ACTOR_SQL,
            &[
                JsValue::from_str(&sha256_hex(api_key.as_bytes())),
                number(now_ms),
            ],
        )
        .await
    }

    pub(crate) async fn request_session_key(
        &self,
        actor: &LegacyMobileSessionActorV1,
        now_ms: i64,
    ) -> MobileResult<String> {
        let operation_id = uuid::Uuid::now_v7().to_string();
        let audit_id = uuid::Uuid::now_v7().to_string();
        let subject_digest = sha256_hex(actor.legacy_user_id.as_bytes());
        let (api_key, key_row_id, key_digest, prior_count) =
            self.key_material(&actor.mapped_user_id).await?;
        let statements = vec![
            self.statement(
                ACTOR_AUTHORITY_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(&actor.legacy_user_id),
                ],
            )?,
            self.statement(
                MOBILE_KEYS_DELETE_SQL,
                &[JsValue::from_str(&actor.mapped_user_id)],
            )?,
            self.statement(
                MOBILE_KEY_INSERT_SQL,
                &[
                    JsValue::from_str(&key_row_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(&key_digest),
                    number(now_ms),
                ],
            )?,
            self.operation(
                &operation_id,
                "session_request",
                Some(&actor.mapped_user_id),
                &subject_digest,
                "not_requested",
                now_ms,
            )?,
            self.receipt(
                &operation_id,
                "api_key_replaced",
                Some(&actor.mapped_user_id),
                Some(&actor.legacy_user_id),
                Some(&key_row_id),
                None,
                prior_count.saturating_add(1),
                now_ms,
            )?,
            self.audit(
                &audit_id,
                &operation_id,
                Some(&actor.mapped_user_id),
                "session_request",
                &subject_digest,
                now_ms,
            )?,
            self.postcondition(
                &operation_id,
                "operation_receipt_audit",
                "session_request",
                "api_key_replaced",
            )?,
            self.statement(
                MOBILE_KEY_POSTCONDITION_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&key_row_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(&key_digest),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&operation_id)])?,
        ];
        self.batch(statements).await?;
        Ok(api_key)
    }

    pub(crate) async fn revoke_session_key(
        &self,
        actor: &LegacyMobileSessionActorV1,
        bearer: &str,
        now_ms: i64,
    ) -> MobileResult<()> {
        let digest = sha256_hex(bearer.as_bytes());
        let operation_id = uuid::Uuid::now_v7().to_string();
        let subject_digest = sha256_hex(actor.legacy_user_id.as_bytes());
        let existing = self
            .first::<CountRowV1>(
                "SELECT COUNT(*) AS key_count FROM auth_api_keys WHERE key_digest = ?1",
                &[JsValue::from_str(&digest)],
            )
            .await?
            .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
        if !(0..=1).contains(&existing.key_count) {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        let statements = vec![
            self.statement(
                ACTOR_AUTHORITY_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&actor.mapped_user_id),
                    JsValue::from_str(&actor.legacy_user_id),
                ],
            )?,
            self.statement(MOBILE_KEY_REVOKE_SQL, &[JsValue::from_str(&digest)])?,
            self.operation(
                &operation_id,
                "session_revoke",
                Some(&actor.mapped_user_id),
                &subject_digest,
                "not_requested",
                now_ms,
            )?,
            self.receipt(
                &operation_id,
                "api_key_revoked",
                Some(&actor.mapped_user_id),
                Some(&actor.legacy_user_id),
                None,
                None,
                existing.key_count,
                now_ms,
            )?,
            self.audit(
                &uuid::Uuid::now_v7().to_string(),
                &operation_id,
                Some(&actor.mapped_user_id),
                "session_revoke",
                &subject_digest,
                now_ms,
            )?,
            self.postcondition(
                &operation_id,
                "operation_receipt_audit",
                "session_revoke",
                "api_key_revoked",
            )?,
            self.statement(
                REVOKE_POSTCONDITION_ASSERT_SQL,
                &[JsValue::from_str(&operation_id), JsValue::from_str(&digest)],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&operation_id)])?,
        ];
        self.batch(statements).await
    }

    async fn user_plan(&self, email: &str) -> MobileResult<UserPlanV1> {
        if let Some(row) = self
            .first::<UserSnapshotRowV1>(USER_SNAPSHOT_SQL, &[JsValue::from_str(email)])
            .await?
        {
            if !matches!(row.has_linked_account, 0 | 1)
                || !matches!(row.has_pending_provisioned_invite, 0 | 1)
                || row.mapped_user_id.len() != 36
                || row
                    .display_name
                    .as_deref()
                    .is_some_and(|name| name.len() > 255)
            {
                return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
            }
            let legacy_user_id = row
                .legacy_user_id
                .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
            LegacyCapNanoId::parse(legacy_user_id.clone())
                .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
            let actor = LegacyMobileSessionActorV1 {
                mapped_user_id: row.mapped_user_id,
                legacy_user_id,
            };
            return Ok(
                if row.has_linked_account == 0 && row.has_pending_provisioned_invite == 1 {
                    UserPlanV1::ExistingProvisioned { actor }
                } else {
                    UserPlanV1::ExistingVisible { actor }
                },
            );
        }
        let pending = self
            .first::<PendingInviteRowV1>(PENDING_INVITE_SQL, &[JsValue::from_str(email)])
            .await?
            .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
        if !matches!(pending.pending_invite, 0 | 1) {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        let legacy_user = random_cap_nanoid()?;
        let organization = if pending.pending_invite == 0 {
            Some((random_cap_nanoid()?, random_cap_nanoid()?))
        } else {
            None
        };
        Ok(UserPlanV1::New {
            actor: LegacyMobileSessionActorV1 {
                mapped_user_id: legacy_user.mapped_uuid().to_string(),
                legacy_user_id: legacy_user.as_str().to_owned(),
            },
            pending_invite: pending.pending_invite == 1,
            organization,
        })
    }

    async fn key_material(&self, actor_id: &str) -> MobileResult<(String, String, String, i64)> {
        let row = self
            .first::<CountRowV1>(MOBILE_KEY_COUNT_SQL, &[JsValue::from_str(actor_id)])
            .await?
            .ok_or(LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
        if row.key_count < 0 || row.key_count > 1_000_000 {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        let api_key = uuid::Uuid::new_v4().to_string();
        Ok((
            api_key.clone(),
            uuid::Uuid::now_v7().to_string(),
            sha256_hex(api_key.as_bytes()),
            row.key_count,
        ))
    }

    async fn actor(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> MobileResult<Option<LegacyMobileSessionActorV1>> {
        let Some(row) = self.first::<ActorRowV1>(sql, bindings).await? else {
            return Ok(None);
        };
        let Some(legacy_user_id) = row.legacy_user_id else {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        };
        if row.mapped_user_id.len() != 36 || LegacyCapNanoId::parse(&legacy_user_id).is_err() {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        Ok(Some(LegacyMobileSessionActorV1 {
            mapped_user_id: row.mapped_user_id,
            legacy_user_id,
        }))
    }

    fn operation(
        &self,
        operation_id: &str,
        action: &str,
        actor_id: Option<&str>,
        subject_digest: &str,
        provider_effect: &str,
        now_ms: i64,
    ) -> MobileResult<D1PreparedStatement> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                JsValue::from_str(operation_id),
                JsValue::from_str(action),
                optional(actor_id),
                JsValue::from_str(subject_digest),
                JsValue::from_str(provider_effect),
                number(now_ms),
            ],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn receipt(
        &self,
        operation_id: &str,
        outcome: &str,
        user_id: Option<&str>,
        legacy_user_id: Option<&str>,
        key_row_id: Option<&str>,
        delivery_id: Option<&str>,
        affected_key_count: i64,
        now_ms: i64,
    ) -> MobileResult<D1PreparedStatement> {
        self.statement(
            RECEIPT_INSERT_SQL,
            &[
                JsValue::from_str(operation_id),
                JsValue::from_str(outcome),
                optional(user_id),
                optional(legacy_user_id),
                optional(key_row_id),
                optional(delivery_id),
                number(affected_key_count),
                number(now_ms),
            ],
        )
    }

    fn audit(
        &self,
        event_id: &str,
        operation_id: &str,
        actor_id: Option<&str>,
        action: &str,
        subject_digest: &str,
        now_ms: i64,
    ) -> MobileResult<D1PreparedStatement> {
        self.statement(
            AUDIT_INSERT_SQL,
            &[
                JsValue::from_str(event_id),
                JsValue::from_str(operation_id),
                optional(actor_id),
                JsValue::from_str(action),
                JsValue::from_str(subject_digest),
                number(now_ms),
            ],
        )
    }

    fn postcondition(
        &self,
        operation_id: &str,
        assertion_kind: &str,
        action: &str,
        outcome: &str,
    ) -> MobileResult<D1PreparedStatement> {
        self.statement(
            POSTCONDITION_ASSERT_SQL,
            &[
                JsValue::from_str(operation_id),
                JsValue::from_str(assertion_kind),
                number(1),
                JsValue::from_str(action),
                JsValue::from_str(outcome),
            ],
        )
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> MobileResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Unavailable)
    }

    async fn first<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> MobileResult<Option<T>> {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyMobileSessionRuntimeFailureV1::Unavailable);
        }
        let mut rows = result
            .results::<T>()
            .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Corrupt)?;
        if rows.len() > 1 {
            return Err(LegacyMobileSessionRuntimeFailureV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn run(&self, sql: &str, bindings: &[JsValue]) -> MobileResult<()> {
        self.run_result(sql, bindings).await.map(|_| ())
    }

    async fn run_result(&self, sql: &str, bindings: &[JsValue]) -> MobileResult<D1Result> {
        let result = self
            .statement(sql, bindings)?
            .run()
            .into_send()
            .await
            .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Unavailable)?;
        if result.success() {
            Ok(result)
        } else {
            Err(LegacyMobileSessionRuntimeFailureV1::Unavailable)
        }
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> MobileResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyMobileSessionRuntimeFailureV1::Unavailable)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyMobileSessionRuntimeFailureV1::Unavailable);
        }
        Ok(())
    }
}

fn random_cap_nanoid() -> MobileResult<LegacyCapNanoId> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut bytes = [0_u8; 15];
    getrandom::fill(&mut bytes).map_err(|_| LegacyMobileSessionRuntimeFailureV1::Unavailable)?;
    let value = bytes
        .iter()
        .map(|byte| ALPHABET[usize::from(byte & 31)] as char)
        .collect::<String>();
    LegacyCapNanoId::parse(value).map_err(|_| LegacyMobileSessionRuntimeFailureV1::Corrupt)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn bytes_to_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn optional(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_sql_keeps_one_use_replace_all_and_provider_outboxes() {
        assert!(CHALLENGE_UPSERT_SQL.contains("ON CONFLICT(identifier_digest) DO UPDATE"));
        assert!(CHALLENGE_DELETE_MATCHING_SQL.contains("token_digest = ?2"));
        assert!(MOBILE_KEYS_DELETE_SQL.contains("legacy_source = 'mobile'"));
        assert!(MOBILE_KEY_POSTCONDITION_ASSERT_SQL.contains("= 1"));
        assert!(HANDOFF_INSERT_SQL.contains("auth_delivery_provider_handoffs_v1"));
        assert!(STRIPE_EFFECT_INSERT_SQL.contains("'pending'"));
        assert!(USER_SNAPSHOT_SQL.contains("identity_accounts"));
        assert!(USER_SNAPSHOT_SQL.contains("has_pending_provisioned_invite"));
    }

    #[test]
    fn plaintext_credentials_never_enter_checked_sql() {
        let sql = [
            HANDOFF_INSERT_SQL,
            CHALLENGE_UPSERT_SQL,
            MOBILE_KEY_INSERT_SQL,
            STRIPE_EFFECT_INSERT_SQL,
        ]
        .join("\n");
        assert!(!sql.contains("normalized_email TEXT"));
        assert!(!sql.contains("api_key TEXT"));
        assert!(sql.contains("payload_hex"));
        assert!(sql.contains("key_digest"));
    }

    #[test]
    fn cap_identifier_generation_is_valid_and_collision_safe_mappable() {
        for _ in 0..32 {
            let id = random_cap_nanoid().expect("random Cap NanoID");
            assert_eq!(id.as_str().len(), 15);
            assert_eq!(id.mapped_uuid().get_version_num(), 8);
        }
    }
}
