//! Atomic D1 runtime for the eight user-owned Cap developer-dashboard actions.
//!
//! The compatibility tables remain isolated from Frame's organization-scoped
//! developer ledger. Every product mutation, one-use browser-grant consumption,
//! typed receipt, effect, audit event, and idempotency transition is submitted in
//! one D1 batch. API credentials are generated, hashed, and AEAD-protected by a
//! local key authority; missing, malformed, or zero key material fails closed.

use std::fmt;

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, Tag, aead::AeadInPlace};
use async_trait::async_trait;
use frame_application::{
    LegacyDeveloperActionV1, LegacyDeveloperApiKeyIdV1, LegacyDeveloperAppIdV1,
    LegacyDeveloperAtomicErrorV1, LegacyDeveloperAtomicOutcomeV1, LegacyDeveloperAtomicPortV1,
    LegacyDeveloperAuthorityPostconditionV1, LegacyDeveloperAutoTopUpStateV1,
    LegacyDeveloperBrowserFenceV1, LegacyDeveloperCommandV1, LegacyDeveloperCreditAccountIdV1,
    LegacyDeveloperDomainIdV1, LegacyDeveloperEnvironmentV1, LegacyDeveloperKeyKindV1,
    LegacyDeveloperMutationPostconditionV1, LegacyDeveloperMutationReceiptV1,
    LegacyDeveloperNullableLogoPatchV1, LegacyDeveloperProtectedBlobV1,
    LegacyDeveloperProtectedKeyPairV1, LegacyDeveloperProtectedProvisioningV1,
    LegacyDeveloperRevealedKeyPairV1, LegacyDeveloperSealedKeyReplayV1,
    LegacyDeveloperSecretAuthorityV1, LegacyDeveloperSecretErrorV1,
    LegacyDeveloperSecretGenerationContextV1, LegacyDeveloperStoredKeyV1,
};
use frame_domain::{SessionId, SessionMutationGrantId, TimestampMillis, UserId};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};
use zeroize::{Zeroize, Zeroizing};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_developer_actions/clock_now.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_developer_actions/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_developer_actions/operation_claim.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_developer_actions/operation_complete.sql");
const BROWSER_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/browser_grant_assert.sql");
const BROWSER_GRANT_DELETE_SQL: &str =
    include_str!("../queries/legacy_developer_actions/browser_grant_delete_returning.sql");
const CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/changes_assert.sql");
const APP_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/app_authority_snapshot.sql");
const APP_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/app_authority_assert.sql");
const DOMAIN_TARGET_COUNT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/domain_target_count.sql");
const VIDEO_TARGET_COUNT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/video_target_count.sql");
const CREATE_APP_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/create_app_insert.sql");
const KEY_INSERT_SQL: &str = include_str!("../queries/legacy_developer_actions/key_insert.sql");
const CREDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/credit_insert.sql");
const CREATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/create_postcondition_assert.sql");
const UPDATE_APP_SQL: &str = include_str!("../queries/legacy_developer_actions/update_app.sql");
const UPDATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/update_postcondition_assert.sql");
const REVOKE_ACTIVE_KEYS_SQL: &str =
    include_str!("../queries/legacy_developer_actions/revoke_active_keys.sql");
const DELETE_APP_SQL: &str = include_str!("../queries/legacy_developer_actions/delete_app.sql");
const DELETE_APP_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/delete_app_postcondition_assert.sql");
const DOMAIN_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/domain_insert.sql");
const DOMAIN_ADD_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/domain_add_postcondition_assert.sql");
const DOMAIN_DELETE_SQL: &str =
    include_str!("../queries/legacy_developer_actions/domain_delete.sql");
const DOMAIN_REMOVE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/domain_remove_postcondition_assert.sql");
const REGENERATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/regenerate_postcondition_assert.sql");
const VIDEO_DELETE_SQL: &str = include_str!("../queries/legacy_developer_actions/video_delete.sql");
const VIDEO_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/video_postcondition_assert.sql");
const AUTO_TOP_UP_UPDATE_SQL: &str =
    include_str!("../queries/legacy_developer_actions/auto_top_up_update.sql");
const AUTO_TOP_UP_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_developer_actions/auto_top_up_postcondition_assert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/receipt_insert.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_developer_actions/audit_insert.sql");
const PROOF_INSERT_SQL: &str = include_str!("../queries/legacy_developer_actions/proof_insert.sql");
const DURABLE_RECEIPT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_developer_actions/durable_receipt_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_developer_actions/assertion_cleanup.sql");

const AUTHORITY_SENTINEL: &str = "frame_legacy_developer_authority_v1";
const CONFLICT_SENTINEL: &str = "frame_legacy_developer_conflict_v1";
const CORRUPT_SENTINEL: &str = "frame_legacy_developer_corrupt_v1";
const OPERATION_UNIQUE_SENTINEL: &str =
    "UNIQUE constraint failed: legacy_developer_action_operations_v1";
const DOMAIN_UNIQUE_SENTINEL: &str = "UNIQUE constraint failed: legacy_developer_app_domains_v1";
const KEY_UNIQUE_SENTINEL: &str = "UNIQUE constraint failed: legacy_developer_api_keys_v1";
const CAP_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const ENVELOPE_VERSION: u16 = 1;
const MAX_ENVELOPE_BYTES: usize = 16 * 1024;

type AtomicResult<T> = Result<T, LegacyDeveloperAtomicErrorV1>;

pub(crate) struct D1LegacyDeveloperAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyDeveloperAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
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
            return Err(LegacyDeveloperAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(results)
    }
}

fn map_d1_message(message: &str) -> LegacyDeveloperAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyDeveloperAtomicErrorV1::StaleAuthority
    } else if message.contains(DOMAIN_UNIQUE_SENTINEL) {
        LegacyDeveloperAtomicErrorV1::DuplicateDomain
    } else if message.contains(OPERATION_UNIQUE_SENTINEL)
        || message.contains(KEY_UNIQUE_SENTINEL)
        || message.contains(CONFLICT_SENTINEL)
    {
        LegacyDeveloperAtomicErrorV1::Conflict
    } else if message.contains(CORRUPT_SENTINEL) {
        LegacyDeveloperAtomicErrorV1::Corrupt
    } else {
        LegacyDeveloperAtomicErrorV1::Unavailable
    }
}

/// Local versioned key authority for developer API credentials and replay data.
pub(crate) struct LocalLegacyDeveloperSecretAuthorityV1 {
    material: [u8; 32],
}

impl fmt::Debug for LocalLegacyDeveloperSecretAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalLegacyDeveloperSecretAuthorityV1([redacted])")
    }
}

impl Drop for LocalLegacyDeveloperSecretAuthorityV1 {
    fn drop(&mut self) {
        self.material.zeroize();
    }
}

impl LocalLegacyDeveloperSecretAuthorityV1 {
    pub(crate) fn from_hex(value: &str) -> Result<Self, LegacyDeveloperSecretErrorV1> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(LegacyDeveloperSecretErrorV1::Unavailable);
        }
        let mut material = [0_u8; 32];
        let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(LegacyDeveloperSecretErrorV1::Unavailable);
        }
        for (index, pair) in pairs.iter().enumerate() {
            let (Some(high), Some(low)) = (hex_nibble(pair[0]), hex_nibble(pair[1])) else {
                material.zeroize();
                return Err(LegacyDeveloperSecretErrorV1::Unavailable);
            };
            material[index] = (high << 4) | low;
        }
        if material.iter().all(|byte| *byte == 0) {
            material.zeroize();
            return Err(LegacyDeveloperSecretErrorV1::Unavailable);
        }
        Ok(Self { material })
    }

    fn derived_key(&self, purpose: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"frame-legacy-developer-local-key-v1\0");
        hasher.update((purpose.len() as u64).to_be_bytes());
        hasher.update(purpose);
        hasher.update(self.material);
        hasher.finalize().into()
    }

    /// Digest a presented Cap developer credential with the same local key
    /// authority used when dashboard actions create or rotate that key.
    pub(crate) fn key_digest_for_auth(&self, raw_key: &str) -> String {
        let mut key = self.derived_key(b"key-hash");
        let digest = lower_hex(&hmac_sha256(&key, raw_key.as_bytes()));
        key.zeroize();
        digest
    }

    fn seal(
        &self,
        purpose: &[u8],
        binding: &[u8; 32],
        plaintext: &[u8],
    ) -> Result<String, LegacyDeveloperSecretErrorV1> {
        if plaintext.is_empty() || plaintext.len() > MAX_ENVELOPE_BYTES - 64 {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        let mut key = self.derived_key(purpose);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| LegacyDeveloperSecretErrorV1::Unavailable)?;
        key.zeroize();
        let nonce = random_bytes::<NONCE_BYTES>()?;
        let aad = secret_aad(purpose, binding);
        let mut ciphertext = Zeroizing::new(plaintext.to_vec());
        let tag = cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut ciphertext)
            .map_err(|_| LegacyDeveloperSecretErrorV1::Unavailable)?;
        let mut envelope = Vec::with_capacity(2 + NONCE_BYTES + ciphertext.len() + TAG_BYTES);
        envelope.extend_from_slice(&ENVELOPE_VERSION.to_be_bytes());
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        envelope.extend_from_slice(&tag);
        let encoded = base64_url_encode(&envelope);
        envelope.zeroize();
        Ok(encoded)
    }

    fn open(
        &self,
        purpose: &[u8],
        binding: &[u8; 32],
        encoded: &str,
    ) -> Result<Vec<u8>, LegacyDeveloperSecretErrorV1> {
        let envelope = Zeroizing::new(
            base64_url_decode(encoded).ok_or(LegacyDeveloperSecretErrorV1::InvalidMaterial)?,
        );
        if envelope.len() < 2 + NONCE_BYTES + TAG_BYTES
            || envelope.len() > MAX_ENVELOPE_BYTES
            || u16::from_be_bytes([envelope[0], envelope[1]]) != ENVELOPE_VERSION
        {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        let mut nonce = [0_u8; NONCE_BYTES];
        nonce.copy_from_slice(&envelope[2..2 + NONCE_BYTES]);
        let ciphertext_start = 2 + NONCE_BYTES;
        let tag_start = envelope.len() - TAG_BYTES;
        let tag = Tag::clone_from_slice(&envelope[tag_start..]);
        let mut ciphertext = Zeroizing::new(envelope[ciphertext_start..tag_start].to_vec());
        let mut key = self.derived_key(purpose);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| LegacyDeveloperSecretErrorV1::Unavailable)?;
        key.zeroize();
        let aad = secret_aad(purpose, binding);
        cipher
            .decrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut ciphertext, &tag)
            .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        Ok(std::mem::take(&mut *ciphertext))
    }
}

#[async_trait]
impl LegacyDeveloperSecretAuthorityV1 for LocalLegacyDeveloperSecretAuthorityV1 {
    async fn generate_protected(
        &self,
        context: &LegacyDeveloperSecretGenerationContextV1,
    ) -> Result<LegacyDeveloperProtectedProvisioningV1, LegacyDeveloperSecretErrorV1> {
        if !matches!(
            context.action(),
            LegacyDeveloperActionV1::CreateApp | LegacyDeveloperActionV1::RegenerateKeys
        ) {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        let binding = context.replay_binding();
        let public_legacy_id = random_cap_nanoid(15)?;
        let secret_legacy_id = random_cap_nanoid(15)?;
        let public_id =
            frame_application::LegacyDeveloperApiKeyIdV1::parse(public_legacy_id.clone())
                .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        let secret_id =
            frame_application::LegacyDeveloperApiKeyIdV1::parse(secret_legacy_id.clone())
                .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        let public_raw = Zeroizing::new(format!("cpk_{}", random_cap_nanoid(30)?));
        let secret_raw = Zeroizing::new(format!("csk_{}", random_cap_nanoid(30)?));
        let public_prefix = public_raw[..12].to_owned();
        let secret_prefix = secret_raw[..12].to_owned();
        let mut hash_key = self.derived_key(b"key-hash");
        let public_digest = lower_hex(&hmac_sha256(&hash_key, public_raw.as_bytes()));
        let secret_digest = lower_hex(&hmac_sha256(&hash_key, secret_raw.as_bytes()));
        hash_key.zeroize();
        let public_encrypted = self.seal(b"key-at-rest-public", &binding, public_raw.as_bytes())?;
        let secret_encrypted = self.seal(b"key-at-rest-secret", &binding, secret_raw.as_bytes())?;
        let mut replay_plaintext =
            Zeroizing::new(Vec::with_capacity(public_raw.len() + secret_raw.len() + 1));
        replay_plaintext.extend_from_slice(public_raw.as_bytes());
        replay_plaintext.push(0);
        replay_plaintext.extend_from_slice(secret_raw.as_bytes());
        let sealed_replay = self.seal(b"response-replay", &binding, &replay_plaintext)?;

        let public = LegacyDeveloperStoredKeyV1::new(
            public_id,
            LegacyDeveloperKeyKindV1::Public,
            public_prefix,
            public_digest,
            LegacyDeveloperProtectedBlobV1::new(public_encrypted)?,
        )?;
        let secret = LegacyDeveloperStoredKeyV1::new(
            secret_id,
            LegacyDeveloperKeyKindV1::Secret,
            secret_prefix,
            secret_digest,
            LegacyDeveloperProtectedBlobV1::new(secret_encrypted)?,
        )?;
        let keys = LegacyDeveloperProtectedKeyPairV1::new(
            public,
            secret,
            LegacyDeveloperSealedKeyReplayV1::new(
                LegacyDeveloperProtectedBlobV1::new(sealed_replay)?,
                binding,
            ),
        )?;
        match context.action() {
            LegacyDeveloperActionV1::CreateApp => {
                Ok(LegacyDeveloperProtectedProvisioningV1::CreateApp {
                    app_id: LegacyDeveloperAppIdV1::parse(random_cap_nanoid(15)?)
                        .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?,
                    credit_account_id: LegacyDeveloperCreditAccountIdV1::parse(random_cap_nanoid(
                        15,
                    )?)
                    .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?,
                    keys,
                })
            }
            LegacyDeveloperActionV1::RegenerateKeys => {
                Ok(LegacyDeveloperProtectedProvisioningV1::RegenerateKeys { keys })
            }
            _ => Err(LegacyDeveloperSecretErrorV1::InvalidMaterial),
        }
    }

    async fn reveal(
        &self,
        replay: &LegacyDeveloperSealedKeyReplayV1,
    ) -> Result<LegacyDeveloperRevealedKeyPairV1, LegacyDeveloperSecretErrorV1> {
        let mut plaintext = self.open(
            b"response-replay",
            replay.binding(),
            replay.ciphertext().expose_for_persistence(),
        )?;
        let separator = plaintext
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        let public = String::from_utf8(plaintext[..separator].to_vec())
            .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        let secret = String::from_utf8(plaintext[separator + 1..].to_vec())
            .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        plaintext.zeroize();
        LegacyDeveloperRevealedKeyPairV1::new(public, secret)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    CreateApp,
    UpdateApp,
    DeleteApp,
    AddDomain,
    RemoveDomain,
    RegenerateKeys,
    DeleteVideo,
    UpdateAutoTopUp,
}

impl Action {
    const fn from_command(command: &LegacyDeveloperCommandV1) -> Self {
        match command {
            LegacyDeveloperCommandV1::CreateApp { .. } => Self::CreateApp,
            LegacyDeveloperCommandV1::UpdateApp { .. } => Self::UpdateApp,
            LegacyDeveloperCommandV1::DeleteApp { .. } => Self::DeleteApp,
            LegacyDeveloperCommandV1::AddDomain { .. } => Self::AddDomain,
            LegacyDeveloperCommandV1::RemoveDomain { .. } => Self::RemoveDomain,
            LegacyDeveloperCommandV1::RegenerateKeys { .. } => Self::RegenerateKeys,
            LegacyDeveloperCommandV1::DeleteVideo { .. } => Self::DeleteVideo,
            LegacyDeveloperCommandV1::UpdateAutoTopUp { .. } => Self::UpdateAutoTopUp,
        }
    }

    const fn journal_name(self) -> &'static str {
        match self {
            Self::CreateApp => "legacy.developer.create_app",
            Self::UpdateApp => "legacy.developer.update_app",
            Self::DeleteApp => "legacy.developer.delete_app",
            Self::AddDomain => "legacy.developer.add_domain",
            Self::RemoveDomain => "legacy.developer.remove_domain",
            Self::RegenerateKeys => "legacy.developer.regenerate_keys",
            Self::DeleteVideo => "legacy.developer.delete_video",
            Self::UpdateAutoTopUp => "legacy.developer.update_auto_top_up",
        }
    }

    const fn result_kind(self) -> &'static str {
        match self {
            Self::CreateApp => "app_created",
            Self::UpdateApp => "app_updated",
            Self::DeleteApp => "app_deleted",
            Self::AddDomain => "domain_added",
            Self::RemoveDomain => "domain_delete_attempted",
            Self::RegenerateKeys => "keys_regenerated",
            Self::DeleteVideo => "video_delete_attempted",
            Self::UpdateAutoTopUp => "auto_top_up_updated",
        }
    }

    const fn revalidate(self) -> bool {
        !matches!(self, Self::CreateApp)
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct AppSnapshot {
    id: String,
    legacy_app_id: String,
    owner_id: String,
    name: String,
    environment: String,
    logo_url: Option<String>,
    last_operation_id: Option<String>,
    revision: i64,
    authority_version: i64,
    active_key_count: i64,
    credit_account_id: Option<String>,
    legacy_credit_account_id: Option<String>,
    auto_top_up_enabled: Option<i64>,
    auto_top_up_threshold_microcredits: Option<i64>,
    auto_top_up_amount_cents: Option<i64>,
    credit_revision: Option<i64>,
}

impl AppSnapshot {
    fn validate(&self, command: &LegacyDeveloperCommandV1) -> AtomicResult<()> {
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
        if self.id != app_id.mapped_uuid()
            || self.legacy_app_id != app_id.legacy_value()
            || self.owner_id != command.fence().authority().actor_id().to_string()
            || self.revision < 0
            || self.authority_version < 0
            || !(0..=i64::from(u32::MAX)).contains(&self.active_key_count)
            || !matches!(self.environment.as_str(), "development" | "production")
            || self.name.is_empty()
            || self.name.chars().count() > 255
            || self
                .logo_url
                .as_ref()
                .is_some_and(|value| value.chars().count() > 1024)
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        let credit_fields = [
            self.auto_top_up_enabled,
            self.auto_top_up_threshold_microcredits,
            self.auto_top_up_amount_cents,
            self.credit_revision,
        ];
        if self.credit_account_id.is_some() != self.legacy_credit_account_id.is_some()
            || credit_fields.iter().all(Option::is_some) != self.credit_account_id.is_some()
            || self
                .auto_top_up_enabled
                .is_some_and(|value| !matches!(value, 0 | 1))
            || self
                .auto_top_up_threshold_microcredits
                .is_some_and(|value| value < 0)
            || self
                .auto_top_up_amount_cents
                .is_some_and(|value| !(0..=100_000).contains(&value))
            || self.credit_revision.is_some_and(|value| value < 0)
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn environment(&self) -> AtomicResult<LegacyDeveloperEnvironmentV1> {
        match self.environment.as_str() {
            "development" => Ok(LegacyDeveloperEnvironmentV1::Development),
            "production" => Ok(LegacyDeveloperEnvironmentV1::Production),
            _ => Err(LegacyDeveloperAtomicErrorV1::Corrupt),
        }
    }

    fn active_key_count(&self) -> AtomicResult<u32> {
        u32::try_from(self.active_key_count).map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
    }
}

#[derive(Debug, Deserialize)]
struct TargetCountRow {
    target_count: i64,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    app_id: Option<String>,
    legacy_app_id: Option<String>,
    final_name: Option<String>,
    final_environment: Option<String>,
    final_logo_url: Option<String>,
    update_statement_executed: Option<i64>,
    deleted_at_ms: Option<i64>,
    revoked_active_key_count: Option<i64>,
    active_key_count_after: Option<i64>,
    domain_id: Option<String>,
    legacy_domain_id: Option<String>,
    stored_origin: Option<String>,
    matched_rows: Option<i64>,
    video_id: Option<String>,
    account_present: Option<i64>,
    auto_top_up_enabled: Option<i64>,
    auto_top_up_threshold_microcredits: Option<i64>,
    auto_top_up_amount_cents: Option<i64>,
    credit_account_id: Option<String>,
    public_key_id: Option<String>,
    secret_key_id: Option<String>,
    sealed_key_replay: Option<String>,
    replay_binding: Option<String>,
    public_legacy_key_id: Option<String>,
    public_key_prefix: Option<String>,
    public_key_digest: Option<String>,
    public_encrypted_key: Option<String>,
    secret_legacy_key_id: Option<String>,
    secret_key_prefix: Option<String>,
    secret_key_digest: Option<String>,
    secret_encrypted_key: Option<String>,
    legacy_credit_account_id: Option<String>,
    revalidate_developer_dashboard: Option<i64>,
    revalidation_path: Option<String>,
    audit_count: i64,
    proof_count: i64,
}

#[derive(Debug, Deserialize)]
struct ConsumedProofRow {
    mutation_grant_id: String,
    session_id: String,
    actor_id: String,
}

#[derive(Clone)]
struct ReceiptWire {
    result_kind: &'static str,
    app_id: Option<String>,
    legacy_app_id: Option<String>,
    final_name: Option<String>,
    final_environment: Option<String>,
    final_logo_url: Option<String>,
    update_statement_executed: Option<i64>,
    deleted_at_ms: Option<i64>,
    revoked_active_key_count: Option<i64>,
    active_key_count_after: Option<i64>,
    domain_id: Option<String>,
    legacy_domain_id: Option<String>,
    stored_origin: Option<String>,
    matched_rows: Option<i64>,
    video_id: Option<String>,
    account_present: Option<i64>,
    auto_top_up_enabled: Option<i64>,
    auto_top_up_threshold_microcredits: Option<i64>,
    auto_top_up_amount_cents: Option<i64>,
    credit_account_id: Option<String>,
    public_key_id: Option<String>,
    secret_key_id: Option<String>,
    sealed_key_replay: Option<String>,
    replay_binding: Option<String>,
}

impl fmt::Debug for ReceiptWire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiptWire")
            .field("result_kind", &self.result_kind)
            .field("protected_fields", &"<redacted>")
            .finish_non_exhaustive()
    }
}

struct MutationPlan {
    operation_id: String,
    audit_id: String,
    action: Action,
    now_ms: i64,
    snapshot: Option<AppSnapshot>,
    target_count: u8,
    provisioning: Option<LegacyDeveloperProtectedProvisioningV1>,
    generated_domain: Option<LegacyDeveloperDomainIdV1>,
    receipt: LegacyDeveloperMutationReceiptV1,
    wire: ReceiptWire,
}

#[derive(Debug, Clone, Copy)]
struct ConsumedProof {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

fn random_bytes<const N: usize>() -> Result<[u8; N], LegacyDeveloperSecretErrorV1> {
    let mut value = [0_u8; N];
    getrandom::fill(&mut value).map_err(|_| LegacyDeveloperSecretErrorV1::Unavailable)?;
    Ok(value)
}

fn random_cap_nanoid(length: usize) -> Result<String, LegacyDeveloperSecretErrorV1> {
    let mut output = String::with_capacity(length);
    while output.len() < length {
        let random = random_bytes::<32>()?;
        for byte in random {
            if output.len() == length {
                break;
            }
            // Thirty-two symbols divide the byte space exactly, so this has no
            // modulo bias while retaining Cap's lowercase NanoID alphabet.
            output.push(char::from(CAP_ALPHABET[usize::from(byte & 0x1f)]));
        }
    }
    Ok(output)
}

fn secret_aad(purpose: &[u8], binding: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64 + purpose.len());
    aad.extend_from_slice(b"frame.legacy-developer.envelope.v1\0");
    aad.extend_from_slice(&(purpose.len() as u64).to_be_bytes());
    aad.extend_from_slice(purpose);
    aad.extend_from_slice(binding);
    aad
}

fn base64_url_encode(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    let (chunks, remainder) = value.as_chunks::<3>();
    for chunk in chunks {
        output.push(char::from(ALPHABET[usize::from(chunk[0] >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from(((chunk[0] & 0x03) << 4) | (chunk[1] >> 4))],
        ));
        output.push(char::from(
            ALPHABET[usize::from(((chunk[1] & 0x0f) << 2) | (chunk[2] >> 6))],
        ));
        output.push(char::from(ALPHABET[usize::from(chunk[2] & 0x3f)]));
    }
    match remainder {
        [one] => {
            output.push(char::from(ALPHABET[usize::from(one >> 2)]));
            output.push(char::from(ALPHABET[usize::from((one & 0x03) << 4)]));
        }
        [one, two] => {
            output.push(char::from(ALPHABET[usize::from(one >> 2)]));
            output.push(char::from(
                ALPHABET[usize::from(((one & 0x03) << 4) | (two >> 4))],
            ));
            output.push(char::from(ALPHABET[usize::from((two & 0x0f) << 2)]));
        }
        _ => {}
    }
    output
}

fn base64_url_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3 + 2);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in value.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits).saturating_sub(1);
        }
    }
    if bits > 0 && accumulator != 0 {
        return None;
    }
    (base64_url_encode(&output) == value).then_some(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
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

fn decode_sha256(value: &str) -> AtomicResult<[u8; 32]> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    let mut output = [0_u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    for (index, pair) in pairs.iter().enumerate() {
        let (Some(high), Some(low)) = (hex_nibble(pair[0]), hex_nibble(pair[1])) else {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        };
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_BYTES: usize = 64;
    let mut normalized = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for ((inner, outer), byte) in inner_pad
        .iter_mut()
        .zip(outer_pad.iter_mut())
        .zip(normalized)
    {
        *inner ^= byte;
        *outer ^= byte;
    }
    normalized.zeroize();
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    inner_pad.zeroize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer_pad.zeroize();
    outer.finalize().into()
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

fn operation_key_digest(actor_id: &str, action: Action, raw_key: &str) -> String {
    digest_fields(
        b"frame.legacy-developer.operation-key.v1\0",
        &[actor_id, action.journal_name(), raw_key],
    )
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

fn authority_for(command: &LegacyDeveloperCommandV1) -> LegacyDeveloperAuthorityPostconditionV1 {
    let actor_id = command.fence().authority().actor_id();
    match command.app_id() {
        Some(app_id) => LegacyDeveloperAuthorityPostconditionV1::ExistingLiveAppOwnedByActor {
            app_id: app_id.clone(),
            owner_id: actor_id,
        },
        None => LegacyDeveloperAuthorityPostconditionV1::NewAppOwnedByActor { actor_id },
    }
}

#[allow(clippy::too_many_lines)]
fn build_plan(
    command: &LegacyDeveloperCommandV1,
    now_ms: i64,
    snapshot: Option<AppSnapshot>,
    target_count: u8,
    provisioning: Option<LegacyDeveloperProtectedProvisioningV1>,
    generated_domain: Option<LegacyDeveloperDomainIdV1>,
) -> AtomicResult<MutationPlan> {
    let operation_id = Uuid::now_v7().to_string();
    let audit_id = Uuid::now_v7().to_string();
    let actor_id = command.fence().authority().actor_id();

    let (mutation, wire) = match command {
        LegacyDeveloperCommandV1::CreateApp {
            name, environment, ..
        } => {
            let provisioning = provisioning
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            let LegacyDeveloperProtectedProvisioningV1::CreateApp {
                app_id,
                credit_account_id,
                keys,
            } = provisioning
            else {
                return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
            };
            let auto_top_up = LegacyDeveloperAutoTopUpStateV1::new(false, 0, 0)?;
            (
                LegacyDeveloperMutationPostconditionV1::AppCreated {
                    owner_id: actor_id,
                    stored_name: name.clone(),
                    environment: *environment,
                    provisioning: provisioning.clone(),
                    active_key_count_after: 2,
                    credit_account_owner_id: actor_id,
                    credit_balance_micro_credits: 0,
                    auto_top_up,
                },
                ReceiptWire {
                    result_kind: Action::CreateApp.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: Some(app_id.legacy_value().to_owned()),
                    final_name: Some(name.clone()),
                    final_environment: Some(environment.stable_code().to_owned()),
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: None,
                    revoked_active_key_count: None,
                    active_key_count_after: Some(2),
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: None,
                    video_id: None,
                    account_present: Some(1),
                    auto_top_up_enabled: Some(0),
                    auto_top_up_threshold_microcredits: Some(0),
                    auto_top_up_amount_cents: Some(0),
                    credit_account_id: Some(credit_account_id.mapped_uuid()),
                    public_key_id: Some(keys.public_key().key_id().mapped_uuid()),
                    secret_key_id: Some(keys.secret_key().key_id().mapped_uuid()),
                    sealed_key_replay: Some(
                        keys.replay()
                            .ciphertext()
                            .expose_for_persistence()
                            .to_owned(),
                    ),
                    replay_binding: Some(lower_hex(keys.replay().binding())),
                },
            )
        }
        LegacyDeveloperCommandV1::UpdateApp { app_id, patch, .. } => {
            let snapshot = snapshot
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            let final_name = patch.name().unwrap_or(&snapshot.name).to_owned();
            let final_environment = patch.environment().unwrap_or(snapshot.environment()?);
            let final_logo_url = match patch.logo_url() {
                LegacyDeveloperNullableLogoPatchV1::Missing => snapshot.logo_url.clone(),
                LegacyDeveloperNullableLogoPatchV1::Null => None,
                LegacyDeveloperNullableLogoPatchV1::Value(value) => Some(value.clone()),
            };
            let executed = !patch.is_empty();
            (
                LegacyDeveloperMutationPostconditionV1::AppUpdated {
                    app_id: app_id.clone(),
                    final_name: final_name.clone(),
                    final_environment,
                    final_logo_url: final_logo_url.clone(),
                    update_statement_executed: executed,
                },
                ReceiptWire {
                    result_kind: Action::UpdateApp.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: Some(final_name),
                    final_environment: Some(final_environment.stable_code().to_owned()),
                    final_logo_url,
                    update_statement_executed: Some(i64::from(executed)),
                    deleted_at_ms: None,
                    revoked_active_key_count: None,
                    active_key_count_after: None,
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: None,
                    video_id: None,
                    account_present: None,
                    auto_top_up_enabled: None,
                    auto_top_up_threshold_microcredits: None,
                    auto_top_up_amount_cents: None,
                    credit_account_id: None,
                    public_key_id: None,
                    secret_key_id: None,
                    sealed_key_replay: None,
                    replay_binding: None,
                },
            )
        }
        LegacyDeveloperCommandV1::DeleteApp { app_id, .. } => {
            let snapshot = snapshot
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            (
                LegacyDeveloperMutationPostconditionV1::AppDeleted {
                    app_id: app_id.clone(),
                    deleted_at: TimestampMillis::new(now_ms)
                        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
                    revoked_active_key_count: snapshot.active_key_count()?,
                    active_key_count_after: 0,
                },
                ReceiptWire {
                    result_kind: Action::DeleteApp.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: None,
                    final_environment: None,
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: Some(now_ms),
                    revoked_active_key_count: Some(snapshot.active_key_count),
                    active_key_count_after: Some(0),
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: None,
                    video_id: None,
                    account_present: None,
                    auto_top_up_enabled: None,
                    auto_top_up_threshold_microcredits: None,
                    auto_top_up_amount_cents: None,
                    credit_account_id: None,
                    public_key_id: None,
                    secret_key_id: None,
                    sealed_key_replay: None,
                    replay_binding: None,
                },
            )
        }
        LegacyDeveloperCommandV1::AddDomain {
            app_id,
            normalized_origin,
            ..
        } => {
            let domain_id = generated_domain
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            (
                LegacyDeveloperMutationPostconditionV1::DomainAdded {
                    app_id: app_id.clone(),
                    domain_id: domain_id.clone(),
                    stored_origin: normalized_origin.clone(),
                },
                ReceiptWire {
                    result_kind: Action::AddDomain.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: None,
                    final_environment: None,
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: None,
                    revoked_active_key_count: None,
                    active_key_count_after: None,
                    domain_id: Some(domain_id.mapped_uuid()),
                    legacy_domain_id: Some(domain_id.legacy_value().to_owned()),
                    stored_origin: Some(normalized_origin.clone()),
                    matched_rows: None,
                    video_id: None,
                    account_present: None,
                    auto_top_up_enabled: None,
                    auto_top_up_threshold_microcredits: None,
                    auto_top_up_amount_cents: None,
                    credit_account_id: None,
                    public_key_id: None,
                    secret_key_id: None,
                    sealed_key_replay: None,
                    replay_binding: None,
                },
            )
        }
        LegacyDeveloperCommandV1::RemoveDomain {
            app_id, domain_id, ..
        } => (
            LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                app_id: app_id.clone(),
                domain_id: domain_id.clone(),
                matched_rows: target_count,
            },
            ReceiptWire {
                result_kind: Action::RemoveDomain.result_kind(),
                app_id: Some(app_id.mapped_uuid()),
                legacy_app_id: None,
                final_name: None,
                final_environment: None,
                final_logo_url: None,
                update_statement_executed: None,
                deleted_at_ms: None,
                revoked_active_key_count: None,
                active_key_count_after: None,
                domain_id: Some(domain_id.mapped_uuid()),
                legacy_domain_id: None,
                stored_origin: None,
                matched_rows: Some(i64::from(target_count)),
                video_id: None,
                account_present: None,
                auto_top_up_enabled: None,
                auto_top_up_threshold_microcredits: None,
                auto_top_up_amount_cents: None,
                credit_account_id: None,
                public_key_id: None,
                secret_key_id: None,
                sealed_key_replay: None,
                replay_binding: None,
            },
        ),
        LegacyDeveloperCommandV1::RegenerateKeys { app_id, .. } => {
            let snapshot = snapshot
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            let provisioning = provisioning
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            let LegacyDeveloperProtectedProvisioningV1::RegenerateKeys { keys } = provisioning
            else {
                return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
            };
            (
                LegacyDeveloperMutationPostconditionV1::KeysRegenerated {
                    app_id: app_id.clone(),
                    revoked_active_key_count: snapshot.active_key_count()?,
                    active_key_count_after: 2,
                    provisioning: provisioning.clone(),
                },
                ReceiptWire {
                    result_kind: Action::RegenerateKeys.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: None,
                    final_environment: None,
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: None,
                    revoked_active_key_count: Some(snapshot.active_key_count),
                    active_key_count_after: Some(2),
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: None,
                    video_id: None,
                    account_present: None,
                    auto_top_up_enabled: None,
                    auto_top_up_threshold_microcredits: None,
                    auto_top_up_amount_cents: None,
                    credit_account_id: None,
                    public_key_id: Some(keys.public_key().key_id().mapped_uuid()),
                    secret_key_id: Some(keys.secret_key().key_id().mapped_uuid()),
                    sealed_key_replay: Some(
                        keys.replay()
                            .ciphertext()
                            .expose_for_persistence()
                            .to_owned(),
                    ),
                    replay_binding: Some(lower_hex(keys.replay().binding())),
                },
            )
        }
        LegacyDeveloperCommandV1::DeleteVideo {
            app_id, video_id, ..
        } => {
            let deleted_at = (target_count == 1)
                .then(|| TimestampMillis::new(now_ms))
                .transpose()
                .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
            (
                LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted {
                    app_id: app_id.clone(),
                    video_id: video_id.clone(),
                    matched_rows: target_count,
                    deleted_at,
                },
                ReceiptWire {
                    result_kind: Action::DeleteVideo.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: None,
                    final_environment: None,
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: (target_count == 1).then_some(now_ms),
                    revoked_active_key_count: None,
                    active_key_count_after: None,
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: Some(i64::from(target_count)),
                    video_id: Some(video_id.mapped_uuid()),
                    account_present: None,
                    auto_top_up_enabled: None,
                    auto_top_up_threshold_microcredits: None,
                    auto_top_up_amount_cents: None,
                    credit_account_id: None,
                    public_key_id: None,
                    secret_key_id: None,
                    sealed_key_replay: None,
                    replay_binding: None,
                },
            )
        }
        LegacyDeveloperCommandV1::UpdateAutoTopUp { app_id, patch, .. } => {
            let snapshot = snapshot
                .as_ref()
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
            let account_state = if snapshot.credit_account_id.is_some() {
                let threshold = patch.threshold_micro_credits().map_or_else(
                    || {
                        u64::try_from(
                            snapshot
                                .auto_top_up_threshold_microcredits
                                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
                        )
                        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
                    },
                    Ok,
                )?;
                let amount = patch.amount_cents().map_or_else(
                    || {
                        u32::try_from(
                            snapshot
                                .auto_top_up_amount_cents
                                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
                        )
                        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
                    },
                    Ok,
                )?;
                Some(LegacyDeveloperAutoTopUpStateV1::new(
                    patch.enabled(),
                    threshold,
                    amount,
                )?)
            } else {
                None
            };
            (
                LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                    app_id: app_id.clone(),
                    account_state: account_state.clone(),
                },
                ReceiptWire {
                    result_kind: Action::UpdateAutoTopUp.result_kind(),
                    app_id: Some(app_id.mapped_uuid()),
                    legacy_app_id: None,
                    final_name: None,
                    final_environment: None,
                    final_logo_url: None,
                    update_statement_executed: None,
                    deleted_at_ms: None,
                    revoked_active_key_count: None,
                    active_key_count_after: None,
                    domain_id: None,
                    legacy_domain_id: None,
                    stored_origin: None,
                    matched_rows: None,
                    video_id: None,
                    account_present: Some(i64::from(account_state.is_some())),
                    auto_top_up_enabled: account_state
                        .as_ref()
                        .map(|state| i64::from(state.enabled())),
                    auto_top_up_threshold_microcredits: account_state.as_ref().map(|state| {
                        i64::try_from(state.threshold_micro_credits())
                            .expect("validated ledger amount fits i64")
                    }),
                    auto_top_up_amount_cents: account_state
                        .as_ref()
                        .map(|state| i64::from(state.amount_cents())),
                    credit_account_id: snapshot.credit_account_id.clone(),
                    public_key_id: None,
                    secret_key_id: None,
                    sealed_key_replay: None,
                    replay_binding: None,
                },
            )
        }
    };
    let receipt = LegacyDeveloperMutationReceiptV1::new(command, authority_for(command), mutation)?;
    Ok(MutationPlan {
        operation_id,
        audit_id,
        action: Action::from_command(command),
        now_ms,
        snapshot,
        target_count,
        provisioning,
        generated_domain,
        receipt,
        wire,
    })
}

impl D1LegacyDeveloperAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.remove(0).now_ms;
        TimestampMillis::new(now_ms).map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        Ok(now_ms)
    }

    async fn operation(
        &self,
        actor_id: &str,
        action: Action,
        key_digest: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![js(actor_id), js(action.journal_name()), js(key_digest)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn app_snapshot(&self, command: &LegacyDeveloperCommandV1) -> AtomicResult<AppSnapshot> {
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
        let actor_id = command.fence().authority().actor_id().to_string();
        let mut rows = self
            .rows::<AppSnapshot>(
                APP_AUTHORITY_SNAPSHOT_SQL,
                vec![js(&app_id.mapped_uuid()), js(&actor_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        let snapshot = rows
            .pop()
            .ok_or(LegacyDeveloperAtomicErrorV1::StaleAuthority)?;
        snapshot.validate(command)?;
        Ok(snapshot)
    }

    async fn target_count(&self, command: &LegacyDeveloperCommandV1) -> AtomicResult<u8> {
        let (sql, target_id, app_id) = match command {
            LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            } => (
                DOMAIN_TARGET_COUNT_SQL,
                domain_id.mapped_uuid(),
                app_id.mapped_uuid(),
            ),
            LegacyDeveloperCommandV1::DeleteVideo {
                app_id, video_id, ..
            } => (
                VIDEO_TARGET_COUNT_SQL,
                video_id.mapped_uuid(),
                app_id.mapped_uuid(),
            ),
            _ => return Ok(0),
        };
        let mut rows = self
            .rows::<TargetCountRow>(sql, vec![js(&target_id), js(&app_id)])
            .await?;
        if rows.len() != 1 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        let count = u8::try_from(rows.remove(0).target_count)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        if count > 1 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(count)
    }

    fn browser_grant_assertion(
        &self,
        assertion_id: &str,
        fence: LegacyDeveloperBrowserFenceV1,
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
        fence: LegacyDeveloperBrowserFenceV1,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_DELETE_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
            ],
        )
    }

    fn changes_assertion(
        &self,
        operation_id: &str,
        kind: &str,
        expected: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            CHANGES_ASSERT_SQL,
            vec![js(operation_id), js(kind), number(expected)],
        )
    }

    fn app_authority_assertion(
        &self,
        operation_id: &str,
        snapshot: &AppSnapshot,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            APP_AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&snapshot.id),
                js(&snapshot.owner_id),
                number(snapshot.revision),
                number(snapshot.authority_version),
            ],
        )
    }

    fn proof_insert(
        &self,
        fence: LegacyDeveloperBrowserFenceV1,
        action: Action,
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
                js(action.journal_name()),
                js(request_digest),
                js(outcome),
                number(now_ms),
            ],
        )
    }

    fn cleanup(&self, operation_id: &str) -> AtomicResult<D1PreparedStatement> {
        self.statement(ASSERTION_CLEANUP_SQL, vec![js(operation_id)])
    }

    fn key_insert(
        &self,
        operation_id: &str,
        app_id: &str,
        key: &LegacyDeveloperStoredKeyV1,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            KEY_INSERT_SQL,
            vec![
                js(&key.key_id().mapped_uuid()),
                js(key.key_id().legacy_value()),
                js(app_id),
                js(key.kind().stable_code()),
                js(key.key_prefix()),
                js(key.key_hash().expose_for_verification()),
                js(key.encrypted_key().expose_for_persistence()),
                number(now_ms),
                js(operation_id),
            ],
        )
    }

    fn receipt_insert(&self, plan: &MutationPlan) -> AtomicResult<D1PreparedStatement> {
        let wire = &plan.wire;
        self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                js(&plan.operation_id),
                js(wire.result_kind),
                js_opt(wire.app_id.as_deref()),
                js_opt(wire.legacy_app_id.as_deref()),
                js_opt(wire.final_name.as_deref()),
                js_opt(wire.final_environment.as_deref()),
                js_opt(wire.final_logo_url.as_deref()),
                number_opt(wire.update_statement_executed),
                number_opt(wire.deleted_at_ms),
                number_opt(wire.revoked_active_key_count),
                number_opt(wire.active_key_count_after),
                js_opt(wire.domain_id.as_deref()),
                js_opt(wire.legacy_domain_id.as_deref()),
                js_opt(wire.stored_origin.as_deref()),
                number_opt(wire.matched_rows),
                js_opt(wire.video_id.as_deref()),
                number_opt(wire.account_present),
                number_opt(wire.auto_top_up_enabled),
                number_opt(wire.auto_top_up_threshold_microcredits),
                number_opt(wire.auto_top_up_amount_cents),
                js_opt(wire.credit_account_id.as_deref()),
                js_opt(wire.public_key_id.as_deref()),
                js_opt(wire.secret_key_id.as_deref()),
                js_opt(wire.sealed_key_replay.as_deref()),
                js_opt(wire.replay_binding.as_deref()),
                number(plan.now_ms),
            ],
        )
    }

    fn durable_receipt_assertion(
        &self,
        plan: &MutationPlan,
        actor_id: &str,
        request_digest: &str,
        fence: LegacyDeveloperBrowserFenceV1,
        outcome: &str,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            DURABLE_RECEIPT_ASSERT_SQL,
            vec![
                js(&plan.operation_id),
                js(actor_id),
                js(plan.action.journal_name()),
                js(request_digest),
                js(plan.wire.result_kind),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(outcome),
            ],
        )
    }

    #[allow(clippy::too_many_lines)]
    fn append_mutation_statements(
        &self,
        command: &LegacyDeveloperCommandV1,
        plan: &MutationPlan,
        statements: &mut Vec<D1PreparedStatement>,
    ) -> AtomicResult<()> {
        let actor_id = command.fence().authority().actor_id().to_string();
        match command {
            LegacyDeveloperCommandV1::CreateApp {
                name, environment, ..
            } => {
                let LegacyDeveloperProtectedProvisioningV1::CreateApp {
                    app_id,
                    credit_account_id,
                    keys,
                } = plan
                    .provisioning
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?
                else {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                };
                statements.push(self.statement(
                    CREATE_APP_INSERT_SQL,
                    vec![
                        js(&app_id.mapped_uuid()),
                        js(app_id.legacy_value()),
                        js(&actor_id),
                        js(name),
                        js(environment.stable_code()),
                        number(plan.now_ms),
                        js(&plan.operation_id),
                    ],
                )?);
                statements.push(self.changes_assertion(&plan.operation_id, "app_mutated", 1)?);
                statements.push(self.key_insert(
                    &plan.operation_id,
                    &app_id.mapped_uuid(),
                    keys.public_key(),
                    plan.now_ms,
                )?);
                statements.push(self.key_insert(
                    &plan.operation_id,
                    &app_id.mapped_uuid(),
                    keys.secret_key(),
                    plan.now_ms,
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "key_rows_mutated",
                    1,
                )?);
                statements.push(self.statement(
                    CREDIT_INSERT_SQL,
                    vec![
                        js(&credit_account_id.mapped_uuid()),
                        js(credit_account_id.legacy_value()),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(plan.now_ms),
                        js(&plan.operation_id),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "account_mutated",
                    1,
                )?);
                statements.push(self.statement(
                    CREATE_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(app_id.legacy_value()),
                        js(&actor_id),
                        js(name),
                        js(environment.stable_code()),
                        js(&credit_account_id.mapped_uuid()),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::UpdateApp { app_id, patch, .. } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                let logo_present = !matches!(
                    patch.logo_url(),
                    LegacyDeveloperNullableLogoPatchV1::Missing
                );
                let logo_value = match patch.logo_url() {
                    LegacyDeveloperNullableLogoPatchV1::Value(value) => Some(value.as_str()),
                    LegacyDeveloperNullableLogoPatchV1::Missing
                    | LegacyDeveloperNullableLogoPatchV1::Null => None,
                };
                let executed = !patch.is_empty();
                statements.push(self.statement(
                    UPDATE_APP_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                        number(i64::from(patch.name().is_some())),
                        js_opt(patch.name()),
                        number(i64::from(patch.environment().is_some())),
                        js_opt(patch.environment().map(|value| value.stable_code())),
                        number(i64::from(logo_present)),
                        js_opt(logo_value),
                        number(plan.now_ms),
                        number(i64::from(executed)),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "app_mutated",
                    i64::from(executed),
                )?);
                let final_revision = if executed {
                    snapshot
                        .revision
                        .checked_add(1)
                        .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?
                } else {
                    snapshot.revision
                };
                let last_operation = if executed {
                    Some(plan.operation_id.as_str())
                } else {
                    snapshot.last_operation_id.as_deref()
                };
                statements.push(self.statement(
                    UPDATE_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        js(plan.wire.final_name.as_deref().ok_or(
                            LegacyDeveloperAtomicErrorV1::Corrupt,
                        )?),
                        js(plan.wire.final_environment.as_deref().ok_or(
                            LegacyDeveloperAtomicErrorV1::Corrupt,
                        )?),
                        js_opt(plan.wire.final_logo_url.as_deref()),
                        number(final_revision),
                        number(snapshot.authority_version),
                        js_opt(last_operation),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::DeleteApp { app_id, .. } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                statements.push(self.statement(
                    REVOKE_ACTIVE_KEYS_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(plan.now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "key_rows_mutated",
                    snapshot.active_key_count,
                )?);
                statements.push(self.statement(
                    DELETE_APP_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                        number(plan.now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(&plan.operation_id, "app_mutated", 1)?);
                statements.push(self.statement(
                    DELETE_APP_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(plan.now_ms),
                        number(snapshot.revision.checked_add(1).ok_or(
                            LegacyDeveloperAtomicErrorV1::Corrupt,
                        )?),
                        number(snapshot.authority_version.checked_add(1).ok_or(
                            LegacyDeveloperAtomicErrorV1::Corrupt,
                        )?),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::AddDomain {
                app_id,
                normalized_origin,
                ..
            } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                let domain_id = plan
                    .generated_domain
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                statements.push(self.statement(
                    DOMAIN_INSERT_SQL,
                    vec![
                        js(&domain_id.mapped_uuid()),
                        js(domain_id.legacy_value()),
                        js(&app_id.mapped_uuid()),
                        js(normalized_origin),
                        number(plan.now_ms),
                        js(&plan.operation_id),
                        js(&actor_id),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                    ],
                )?);
                statements.push(self.changes_assertion(&plan.operation_id, "domain_mutated", 1)?);
                statements.push(self.statement(
                    DOMAIN_ADD_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&domain_id.mapped_uuid()),
                        js(domain_id.legacy_value()),
                        js(&app_id.mapped_uuid()),
                        js(normalized_origin),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                statements.push(self.statement(
                    DOMAIN_DELETE_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&domain_id.mapped_uuid()),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "domain_mutated",
                    i64::from(plan.target_count),
                )?);
                statements.push(self.statement(
                    DOMAIN_REMOVE_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&domain_id.mapped_uuid()),
                        js(&app_id.mapped_uuid()),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::RegenerateKeys { app_id, .. } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                let LegacyDeveloperProtectedProvisioningV1::RegenerateKeys { keys } = plan
                    .provisioning
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?
                else {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                };
                statements.push(self.statement(
                    REVOKE_ACTIVE_KEYS_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(plan.now_ms),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "key_rows_mutated",
                    snapshot.active_key_count,
                )?);
                statements.push(self.key_insert(
                    &plan.operation_id,
                    &app_id.mapped_uuid(),
                    keys.public_key(),
                    plan.now_ms,
                )?);
                statements.push(self.key_insert(
                    &plan.operation_id,
                    &app_id.mapped_uuid(),
                    keys.secret_key(),
                    plan.now_ms,
                )?);
                statements.push(self.statement(
                    REGENERATE_POSTCONDITION_SQL,
                    vec![js(&plan.operation_id), js(&app_id.mapped_uuid())],
                )?);
            }
            LegacyDeveloperCommandV1::DeleteVideo {
                app_id, video_id, ..
            } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                statements.push(self.statement(
                    VIDEO_DELETE_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&video_id.mapped_uuid()),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(plan.now_ms),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "video_mutated",
                    i64::from(plan.target_count),
                )?);
                statements.push(self.statement(
                    VIDEO_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&video_id.mapped_uuid()),
                        js(&app_id.mapped_uuid()),
                        number(i64::from(plan.target_count)),
                        number_opt((plan.target_count == 1).then_some(plan.now_ms)),
                    ],
                )?);
            }
            LegacyDeveloperCommandV1::UpdateAutoTopUp { app_id, patch, .. } => {
                let snapshot = plan
                    .snapshot
                    .as_ref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                let account_present = i64::from(snapshot.credit_account_id.is_some());
                statements.push(self.statement(
                    AUTO_TOP_UP_UPDATE_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        js(&actor_id),
                        number(i64::from(patch.enabled())),
                        number(i64::from(patch.threshold_micro_credits().is_some())),
                        number_opt(
                            patch
                                .threshold_micro_credits()
                                .and_then(|value| i64::try_from(value).ok()),
                        ),
                        number(i64::from(patch.amount_cents().is_some())),
                        number_opt(patch.amount_cents().map(i64::from)),
                        number(plan.now_ms),
                        number_opt(snapshot.credit_revision),
                        number(snapshot.revision),
                        number(snapshot.authority_version),
                    ],
                )?);
                statements.push(self.changes_assertion(
                    &plan.operation_id,
                    "account_mutated",
                    account_present,
                )?);
                statements.push(self.statement(
                    AUTO_TOP_UP_POSTCONDITION_SQL,
                    vec![
                        js(&plan.operation_id),
                        js(&app_id.mapped_uuid()),
                        number(account_present),
                        js(&actor_id),
                        number_opt(plan.wire.auto_top_up_enabled),
                        number_opt(plan.wire.auto_top_up_threshold_microcredits),
                        number_opt(plan.wire.auto_top_up_amount_cents),
                        number_opt(
                            snapshot
                                .credit_revision
                                .and_then(|value| value.checked_add(1)),
                        ),
                    ],
                )?);
            }
        }
        Ok(())
    }
}

fn decode_consumed_proof(
    result: &D1Result,
    fence: LegacyDeveloperBrowserFenceV1,
) -> AtomicResult<ConsumedProof> {
    let mut rows = result
        .results::<ConsumedProofRow>()
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
    if rows.len() != 1 {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    let row = rows.remove(0);
    let proof = ConsumedProof {
        mutation_grant_id: SessionMutationGrantId::parse(&row.mutation_grant_id)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
        session_id: SessionId::parse(&row.session_id)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
        actor_id: UserId::parse(&row.actor_id)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
    };
    if proof.mutation_grant_id != fence.mutation_grant_id()
        || proof.session_id != fence.session_id()
        || proof.actor_id != fence.actor_id()
    {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    Ok(proof)
}

impl OperationRow {
    fn validate_identity(&self) -> AtomicResult<()> {
        if Uuid::parse_str(&self.operation_id).is_err()
            || decode_sha256(&self.request_digest).is_err()
            || self.audit_count < 0
            || self.proof_count < 0
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn clean_claim(&self) -> bool {
        self.state == "claimed"
            && self.result_kind.is_none()
            && self.app_id.is_none()
            && self.legacy_app_id.is_none()
            && self.final_name.is_none()
            && self.final_environment.is_none()
            && self.final_logo_url.is_none()
            && self.update_statement_executed.is_none()
            && self.deleted_at_ms.is_none()
            && self.revoked_active_key_count.is_none()
            && self.active_key_count_after.is_none()
            && self.domain_id.is_none()
            && self.legacy_domain_id.is_none()
            && self.stored_origin.is_none()
            && self.matched_rows.is_none()
            && self.video_id.is_none()
            && self.account_present.is_none()
            && self.auto_top_up_enabled.is_none()
            && self.auto_top_up_threshold_microcredits.is_none()
            && self.auto_top_up_amount_cents.is_none()
            && self.credit_account_id.is_none()
            && self.public_key_id.is_none()
            && self.secret_key_id.is_none()
            && self.sealed_key_replay.is_none()
            && self.replay_binding.is_none()
            && self.public_legacy_key_id.is_none()
            && self.public_key_prefix.is_none()
            && self.public_key_digest.is_none()
            && self.public_encrypted_key.is_none()
            && self.secret_legacy_key_id.is_none()
            && self.secret_key_prefix.is_none()
            && self.secret_key_digest.is_none()
            && self.secret_encrypted_key.is_none()
            && self.legacy_credit_account_id.is_none()
            && self.revalidate_developer_dashboard.is_none()
            && self.revalidation_path.is_none()
            && self.audit_count == 0
            && self.proof_count == 0
    }

    fn validate_complete_envelope(&self, action: Action) -> AtomicResult<()> {
        if self.state != "complete"
            || self.result_kind.as_deref() != Some(action.result_kind())
            || self.audit_count != 1
            || self.proof_count < 1
            || self.revalidate_developer_dashboard != Some(i64::from(action.revalidate()))
            || self.revalidation_path.as_deref()
                != action.revalidate().then_some("/dashboard/developers")
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn receipt(
        &self,
        command: &LegacyDeveloperCommandV1,
    ) -> AtomicResult<LegacyDeveloperMutationReceiptV1> {
        let action = Action::from_command(command);
        self.validate_identity()?;
        self.validate_complete_envelope(action)?;
        let actor_id = command.fence().authority().actor_id();
        let mutation = match command {
            LegacyDeveloperCommandV1::CreateApp { .. } => {
                if any_some(&[
                    self.final_logo_url.as_ref(),
                    self.domain_id.as_ref(),
                    self.legacy_domain_id.as_ref(),
                    self.stored_origin.as_ref(),
                    self.video_id.as_ref(),
                ]) || any_some_i64(&[
                    self.update_statement_executed,
                    self.deleted_at_ms,
                    self.revoked_active_key_count,
                    self.matched_rows,
                ]) || self.active_key_count_after != Some(2)
                    || self.account_present != Some(1)
                    || self.auto_top_up_enabled != Some(0)
                    || self.auto_top_up_threshold_microcredits != Some(0)
                    || self.auto_top_up_amount_cents != Some(0)
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                let app_id =
                    parse_legacy_app_pair(self.app_id.as_deref(), self.legacy_app_id.as_deref())?;
                let credit_account_id = parse_legacy_credit_pair(
                    self.credit_account_id.as_deref(),
                    self.legacy_credit_account_id.as_deref(),
                )?;
                let keys = self.protected_keys()?;
                LegacyDeveloperMutationPostconditionV1::AppCreated {
                    owner_id: actor_id,
                    stored_name: self
                        .final_name
                        .clone()
                        .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
                    environment: parse_environment(self.final_environment.as_deref())?,
                    provisioning: LegacyDeveloperProtectedProvisioningV1::CreateApp {
                        app_id,
                        credit_account_id,
                        keys,
                    },
                    active_key_count_after: 2,
                    credit_account_owner_id: actor_id,
                    credit_balance_micro_credits: 0,
                    auto_top_up: LegacyDeveloperAutoTopUpStateV1::new(false, 0, 0)?,
                }
            }
            LegacyDeveloperCommandV1::UpdateApp { app_id, .. } => {
                self.assert_app(app_id)?;
                if self.legacy_app_id.is_some()
                    || any_some(&[
                        self.domain_id.as_ref(),
                        self.legacy_domain_id.as_ref(),
                        self.stored_origin.as_ref(),
                        self.video_id.as_ref(),
                        self.credit_account_id.as_ref(),
                        self.public_key_id.as_ref(),
                        self.secret_key_id.as_ref(),
                        self.sealed_key_replay.as_ref(),
                        self.replay_binding.as_ref(),
                    ])
                    || any_some_i64(&[
                        self.deleted_at_ms,
                        self.revoked_active_key_count,
                        self.active_key_count_after,
                        self.matched_rows,
                        self.account_present,
                        self.auto_top_up_enabled,
                        self.auto_top_up_threshold_microcredits,
                        self.auto_top_up_amount_cents,
                    ])
                    || self.has_joined_key_material()
                    || self.legacy_credit_account_id.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::AppUpdated {
                    app_id: app_id.clone(),
                    final_name: self
                        .final_name
                        .clone()
                        .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
                    final_environment: parse_environment(self.final_environment.as_deref())?,
                    final_logo_url: self.final_logo_url.clone(),
                    update_statement_executed: required_bool(self.update_statement_executed)?,
                }
            }
            LegacyDeveloperCommandV1::DeleteApp { app_id, .. } => {
                self.assert_app(app_id)?;
                if self.legacy_app_id.is_some()
                    || self.final_name.is_some()
                    || self.final_environment.is_some()
                    || self.final_logo_url.is_some()
                    || self.update_statement_executed.is_some()
                    || self.active_key_count_after != Some(0)
                    || any_some(&[
                        self.domain_id.as_ref(),
                        self.legacy_domain_id.as_ref(),
                        self.stored_origin.as_ref(),
                        self.video_id.as_ref(),
                        self.credit_account_id.as_ref(),
                    ])
                    || any_some_i64(&[
                        self.matched_rows,
                        self.account_present,
                        self.auto_top_up_enabled,
                        self.auto_top_up_threshold_microcredits,
                        self.auto_top_up_amount_cents,
                    ])
                    || self.has_key_receipt_material()
                    || self.has_joined_key_material()
                    || self.legacy_credit_account_id.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::AppDeleted {
                    app_id: app_id.clone(),
                    deleted_at: required_timestamp(self.deleted_at_ms)?,
                    revoked_active_key_count: required_u32(self.revoked_active_key_count)?,
                    active_key_count_after: 0,
                }
            }
            LegacyDeveloperCommandV1::AddDomain { app_id, .. } => {
                self.assert_app(app_id)?;
                self.assert_common_sparse()?;
                let legacy_domain_id = self
                    .legacy_domain_id
                    .as_deref()
                    .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?;
                let domain_id = LegacyDeveloperDomainIdV1::parse(legacy_domain_id.to_owned())
                    .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
                if self.domain_id.as_deref() != Some(domain_id.mapped_uuid().as_str())
                    || self.stored_origin.is_none()
                    || self.deleted_at_ms.is_some()
                    || self.matched_rows.is_some()
                    || self.video_id.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::DomainAdded {
                    app_id: app_id.clone(),
                    domain_id,
                    stored_origin: self
                        .stored_origin
                        .clone()
                        .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
                }
            }
            LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            } => {
                self.assert_app(app_id)?;
                self.assert_common_sparse()?;
                if self.domain_id.as_deref() != Some(domain_id.mapped_uuid().as_str())
                    || self.legacy_domain_id.is_some()
                    || self.stored_origin.is_some()
                    || self.deleted_at_ms.is_some()
                    || self.video_id.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                    app_id: app_id.clone(),
                    domain_id: domain_id.clone(),
                    matched_rows: required_u8(self.matched_rows)?,
                }
            }
            LegacyDeveloperCommandV1::RegenerateKeys { app_id, .. } => {
                self.assert_app(app_id)?;
                if self.legacy_app_id.is_some()
                    || self.final_name.is_some()
                    || self.final_environment.is_some()
                    || self.final_logo_url.is_some()
                    || self.update_statement_executed.is_some()
                    || self.deleted_at_ms.is_some()
                    || self.active_key_count_after != Some(2)
                    || any_some(&[
                        self.domain_id.as_ref(),
                        self.legacy_domain_id.as_ref(),
                        self.stored_origin.as_ref(),
                        self.video_id.as_ref(),
                        self.credit_account_id.as_ref(),
                    ])
                    || any_some_i64(&[
                        self.matched_rows,
                        self.account_present,
                        self.auto_top_up_enabled,
                        self.auto_top_up_threshold_microcredits,
                        self.auto_top_up_amount_cents,
                    ])
                    || self.legacy_credit_account_id.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::KeysRegenerated {
                    app_id: app_id.clone(),
                    revoked_active_key_count: required_u32(self.revoked_active_key_count)?,
                    active_key_count_after: 2,
                    provisioning: LegacyDeveloperProtectedProvisioningV1::RegenerateKeys {
                        keys: self.protected_keys()?,
                    },
                }
            }
            LegacyDeveloperCommandV1::DeleteVideo {
                app_id, video_id, ..
            } => {
                self.assert_app(app_id)?;
                self.assert_common_sparse()?;
                if self.video_id.as_deref() != Some(video_id.mapped_uuid().as_str())
                    || self.domain_id.is_some()
                    || self.legacy_domain_id.is_some()
                    || self.stored_origin.is_some()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                let matched_rows = required_u8(self.matched_rows)?;
                let deleted_at = self
                    .deleted_at_ms
                    .map(|value| {
                        TimestampMillis::new(value)
                            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
                    })
                    .transpose()?;
                if (matched_rows == 1) != deleted_at.is_some() {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted {
                    app_id: app_id.clone(),
                    video_id: video_id.clone(),
                    matched_rows,
                    deleted_at,
                }
            }
            LegacyDeveloperCommandV1::UpdateAutoTopUp { app_id, .. } => {
                self.assert_app(app_id)?;
                if self.legacy_app_id.is_some()
                    || self.final_name.is_some()
                    || self.final_environment.is_some()
                    || self.final_logo_url.is_some()
                    || self.update_statement_executed.is_some()
                    || self.deleted_at_ms.is_some()
                    || self.revoked_active_key_count.is_some()
                    || self.active_key_count_after.is_some()
                    || any_some(&[
                        self.domain_id.as_ref(),
                        self.legacy_domain_id.as_ref(),
                        self.stored_origin.as_ref(),
                        self.video_id.as_ref(),
                    ])
                    || self.matched_rows.is_some()
                    || self.has_key_receipt_material()
                    || self.has_joined_key_material()
                {
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                let account_state = match required_bool(self.account_present)? {
                    false => {
                        if self.auto_top_up_enabled.is_some()
                            || self.auto_top_up_threshold_microcredits.is_some()
                            || self.auto_top_up_amount_cents.is_some()
                            || self.credit_account_id.is_some()
                            || self.legacy_credit_account_id.is_some()
                        {
                            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                        }
                        None
                    }
                    true => {
                        parse_legacy_credit_pair(
                            self.credit_account_id.as_deref(),
                            self.legacy_credit_account_id.as_deref(),
                        )?;
                        Some(LegacyDeveloperAutoTopUpStateV1::new(
                            required_bool(self.auto_top_up_enabled)?,
                            required_u64(self.auto_top_up_threshold_microcredits)?,
                            required_u32(self.auto_top_up_amount_cents)?,
                        )?)
                    }
                };
                LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                    app_id: app_id.clone(),
                    account_state,
                }
            }
        };
        LegacyDeveloperMutationReceiptV1::new(command, authority_for(command), mutation)
    }

    fn assert_app(&self, expected: &LegacyDeveloperAppIdV1) -> AtomicResult<()> {
        if self.app_id.as_deref() != Some(expected.mapped_uuid().as_str()) {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn assert_common_sparse(&self) -> AtomicResult<()> {
        if self.legacy_app_id.is_some()
            || self.final_name.is_some()
            || self.final_environment.is_some()
            || self.final_logo_url.is_some()
            || self.update_statement_executed.is_some()
            || self.revoked_active_key_count.is_some()
            || self.active_key_count_after.is_some()
            || self.account_present.is_some()
            || self.auto_top_up_enabled.is_some()
            || self.auto_top_up_threshold_microcredits.is_some()
            || self.auto_top_up_amount_cents.is_some()
            || self.credit_account_id.is_some()
            || self.has_key_receipt_material()
            || self.has_joined_key_material()
            || self.legacy_credit_account_id.is_some()
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn has_key_receipt_material(&self) -> bool {
        self.public_key_id.is_some()
            || self.secret_key_id.is_some()
            || self.sealed_key_replay.is_some()
            || self.replay_binding.is_some()
    }

    fn has_joined_key_material(&self) -> bool {
        any_some(&[
            self.public_legacy_key_id.as_ref(),
            self.public_key_prefix.as_ref(),
            self.public_key_digest.as_ref(),
            self.public_encrypted_key.as_ref(),
            self.secret_legacy_key_id.as_ref(),
            self.secret_key_prefix.as_ref(),
            self.secret_key_digest.as_ref(),
            self.secret_encrypted_key.as_ref(),
        ])
    }

    fn protected_keys(&self) -> AtomicResult<LegacyDeveloperProtectedKeyPairV1> {
        let public_id = LegacyDeveloperApiKeyIdV1::parse(
            required_str(self.public_legacy_key_id.as_deref())?.to_owned(),
        )
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        let secret_id = LegacyDeveloperApiKeyIdV1::parse(
            required_str(self.secret_legacy_key_id.as_deref())?.to_owned(),
        )
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        if self.public_key_id.as_deref() != Some(public_id.mapped_uuid().as_str())
            || self.secret_key_id.as_deref() != Some(secret_id.mapped_uuid().as_str())
        {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        let public = LegacyDeveloperStoredKeyV1::new(
            public_id,
            LegacyDeveloperKeyKindV1::Public,
            required_str(self.public_key_prefix.as_deref())?,
            required_str(self.public_key_digest.as_deref())?,
            LegacyDeveloperProtectedBlobV1::new(required_str(
                self.public_encrypted_key.as_deref(),
            )?)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
        )
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        let secret = LegacyDeveloperStoredKeyV1::new(
            secret_id,
            LegacyDeveloperKeyKindV1::Secret,
            required_str(self.secret_key_prefix.as_deref())?,
            required_str(self.secret_key_digest.as_deref())?,
            LegacyDeveloperProtectedBlobV1::new(required_str(
                self.secret_encrypted_key.as_deref(),
            )?)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
        )
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
        let replay = LegacyDeveloperSealedKeyReplayV1::new(
            LegacyDeveloperProtectedBlobV1::new(required_str(self.sealed_key_replay.as_deref())?)
                .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
            decode_sha256(required_str(self.replay_binding.as_deref())?)?,
        );
        LegacyDeveloperProtectedKeyPairV1::new(public, secret, replay)
            .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
    }
}

fn any_some(values: &[Option<&String>]) -> bool {
    values.iter().any(Option::is_some)
}

fn any_some_i64(values: &[Option<i64>]) -> bool {
    values.iter().any(Option::is_some)
}

fn required_str(value: Option<&str>) -> AtomicResult<&str> {
    value.ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)
}

fn required_bool(value: Option<i64>) -> AtomicResult<bool> {
    match value {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(LegacyDeveloperAtomicErrorV1::Corrupt),
    }
}

fn required_u8(value: Option<i64>) -> AtomicResult<u8> {
    let value = u8::try_from(value.ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
    if value > 1 {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    Ok(value)
}

fn required_u32(value: Option<i64>) -> AtomicResult<u32> {
    u32::try_from(value.ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
}

fn required_u64(value: Option<i64>) -> AtomicResult<u64> {
    u64::try_from(value.ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
}

fn required_timestamp(value: Option<i64>) -> AtomicResult<TimestampMillis> {
    TimestampMillis::new(value.ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)
}

fn parse_environment(value: Option<&str>) -> AtomicResult<LegacyDeveloperEnvironmentV1> {
    match value {
        Some("development") => Ok(LegacyDeveloperEnvironmentV1::Development),
        Some("production") => Ok(LegacyDeveloperEnvironmentV1::Production),
        _ => Err(LegacyDeveloperAtomicErrorV1::Corrupt),
    }
}

fn parse_legacy_app_pair(
    mapped: Option<&str>,
    legacy: Option<&str>,
) -> AtomicResult<LegacyDeveloperAppIdV1> {
    let app_id = LegacyDeveloperAppIdV1::parse(required_str(legacy)?.to_owned())
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
    if mapped != Some(app_id.mapped_uuid().as_str()) {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    Ok(app_id)
}

fn parse_legacy_credit_pair(
    mapped: Option<&str>,
    legacy: Option<&str>,
) -> AtomicResult<LegacyDeveloperCreditAccountIdV1> {
    let account_id = LegacyDeveloperCreditAccountIdV1::parse(required_str(legacy)?.to_owned())
        .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?;
    if mapped != Some(account_id.mapped_uuid().as_str()) {
        return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
    }
    Ok(account_id)
}

impl D1LegacyDeveloperAtomicPortV1<'_> {
    async fn consume_only(
        &self,
        fence: LegacyDeveloperBrowserFenceV1,
        action: Action,
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
            action,
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
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
            fence,
        )
    }

    async fn reject_and_return<T>(
        &self,
        fence: LegacyDeveloperBrowserFenceV1,
        action: Action,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
        error: LegacyDeveloperAtomicErrorV1,
    ) -> AtomicResult<T> {
        self.consume_only(fence, action, related_operation_id, request_digest, outcome)
            .await?;
        Err(error)
    }

    async fn consume_replay(
        &self,
        fence: LegacyDeveloperBrowserFenceV1,
        action: Action,
        operation_id: &str,
        request_digest: &str,
    ) -> AtomicResult<ConsumedProof> {
        let now_ms = self.clock_now().await?;
        let mut statements = vec![self.browser_grant_assertion(operation_id, fence, now_ms)?];
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            action,
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
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
            fence,
        )
    }

    async fn existing_outcome(
        &self,
        command: &LegacyDeveloperCommandV1,
        fence: LegacyDeveloperBrowserFenceV1,
        operation: &OperationRow,
        request_digest: &str,
    ) -> AtomicResult<LegacyDeveloperAtomicOutcomeV1> {
        let action = Action::from_command(command);
        if let Err(error) = operation.validate_identity() {
            let _ = self
                .consume_only(fence, action, None, request_digest, "rejected")
                .await;
            return Err(error);
        }
        if operation.clean_claim() {
            return self
                .reject_and_return(
                    fence,
                    action,
                    Some(&operation.operation_id),
                    request_digest,
                    "in_flight",
                    LegacyDeveloperAtomicErrorV1::InFlight,
                )
                .await;
        }
        let receipt = match operation.receipt(command) {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = self
                    .consume_only(
                        fence,
                        action,
                        Some(&operation.operation_id),
                        request_digest,
                        "rejected",
                    )
                    .await;
                return Err(error);
            }
        };
        let proof = self
            .consume_replay(fence, action, &operation.operation_id, request_digest)
            .await?;
        if proof.actor_id != command.fence().authority().actor_id() {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(LegacyDeveloperAtomicOutcomeV1::Replay(receipt))
    }

    async fn prepare_plan(
        &self,
        command: &LegacyDeveloperCommandV1,
        secrets: &dyn LegacyDeveloperSecretAuthorityV1,
    ) -> AtomicResult<MutationPlan> {
        let snapshot = if command.app_id().is_some() {
            Some(self.app_snapshot(command).await?)
        } else {
            None
        };
        let target_count = self.target_count(command).await?;

        // D1 accepts only fully prepared statements in an atomic batch. A
        // protected candidate therefore has to be generated after the
        // authority snapshot but before the all-or-nothing claim batch. An
        // already established operation is handled before this method, and a
        // losing concurrent candidate is never persisted or returned.
        let provisioning = match command.secret_generation_context() {
            Some(context) => Some(
                secrets
                    .generate_protected(&context)
                    .await
                    .map_err(map_secret_error)?,
            ),
            None => None,
        };
        let generated_domain = if matches!(command, LegacyDeveloperCommandV1::AddDomain { .. }) {
            Some(
                LegacyDeveloperDomainIdV1::parse(random_cap_nanoid(15).map_err(map_secret_error)?)
                    .map_err(|_| LegacyDeveloperAtomicErrorV1::Corrupt)?,
            )
        } else {
            None
        };
        let now_ms = self.clock_now().await?;
        build_plan(
            command,
            now_ms,
            snapshot,
            target_count,
            provisioning,
            generated_domain,
        )
    }

    async fn execute_fresh(
        &self,
        command: &LegacyDeveloperCommandV1,
        fence: LegacyDeveloperBrowserFenceV1,
        key_digest: &str,
        request_digest: &str,
        secrets: &dyn LegacyDeveloperSecretAuthorityV1,
    ) -> AtomicResult<LegacyDeveloperMutationReceiptV1> {
        let plan = self.prepare_plan(command, secrets).await?;
        let actor_id = command.fence().authority().actor_id().to_string();
        let subject_digest = digest_fields(
            b"frame.legacy-developer.subject.v1\0",
            &[
                &actor_id,
                plan.action.journal_name(),
                plan.wire.app_id.as_deref().unwrap_or("new-app"),
                request_digest,
            ],
        );
        let mut statements = vec![
            self.browser_grant_assertion(&plan.operation_id, fence, plan.now_ms)?,
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&plan.operation_id),
                    js(&actor_id),
                    js(plan.action.journal_name()),
                    js(key_digest),
                    js(request_digest),
                    number(plan.now_ms),
                ],
            )?,
        ];
        if let Some(snapshot) = &plan.snapshot {
            statements.push(self.app_authority_assertion(&plan.operation_id, snapshot)?);
        }
        self.append_mutation_statements(command, &plan, &mut statements)?;
        statements.push(self.receipt_insert(&plan)?);
        statements.push(self.changes_assertion(&plan.operation_id, "receipt_inserted", 1)?);
        statements.push(self.statement(
            EFFECT_INSERT_SQL,
            vec![
                js(&plan.operation_id),
                number(i64::from(plan.action.revalidate())),
                js_opt(plan.action.revalidate().then_some("/dashboard/developers")),
                number(plan.now_ms),
            ],
        )?);
        statements.push(self.changes_assertion(&plan.operation_id, "effect_inserted", 1)?);
        statements.push(self.statement(
            AUDIT_INSERT_SQL,
            vec![
                js(&plan.audit_id),
                js(&plan.operation_id),
                js(&actor_id),
                js(plan.action.journal_name()),
                js(&subject_digest),
                number(plan.now_ms),
            ],
        )?);
        statements.push(self.changes_assertion(&plan.operation_id, "audit_inserted", 1)?);
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(&plan.operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            plan.action,
            Some(&plan.operation_id),
            request_digest,
            "applied",
            plan.now_ms,
        )?);
        statements.push(self.changes_assertion(&plan.operation_id, "proof_journaled", 1)?);
        statements.push(self.statement(
            OPERATION_COMPLETE_SQL,
            vec![js(&plan.operation_id), number(plan.now_ms)],
        )?);
        statements.push(self.changes_assertion(&plan.operation_id, "operation_complete", 1)?);
        statements.push(self.durable_receipt_assertion(
            &plan,
            &actor_id,
            request_digest,
            fence,
            "applied",
        )?);
        statements.push(self.cleanup(&plan.operation_id)?);

        let results = self.batch_results(statements).await?;
        let proof = decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyDeveloperAtomicErrorV1::Corrupt)?,
            fence,
        )?;
        if proof.actor_id != command.fence().authority().actor_id() {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        plan.receipt.validate_against(command)?;
        Ok(plan.receipt)
    }

    #[allow(clippy::too_many_arguments)]
    async fn reconcile(
        &self,
        command: &LegacyDeveloperCommandV1,
        fence: LegacyDeveloperBrowserFenceV1,
        actor_id: &str,
        action: Action,
        key_digest: &str,
        request_digest: &str,
        original_error: LegacyDeveloperAtomicErrorV1,
    ) -> AtomicResult<LegacyDeveloperAtomicOutcomeV1> {
        match self.operation(actor_id, action, key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                self.existing_outcome(command, fence, &operation, request_digest)
                    .await
            }
            Ok(Some(operation)) => {
                if operation.validate_identity().is_err() {
                    let _ = self
                        .consume_only(fence, action, None, request_digest, "rejected")
                        .await;
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                self.reject_and_return(
                    fence,
                    action,
                    Some(&operation.operation_id),
                    request_digest,
                    "conflict",
                    LegacyDeveloperAtomicErrorV1::Conflict,
                )
                .await
            }
            Ok(None) => {
                self.reject_and_return(
                    fence,
                    action,
                    None,
                    request_digest,
                    "rejected",
                    original_error,
                )
                .await
            }
            Err(_) => {
                let _ = self
                    .consume_only(fence, action, None, request_digest, "rejected")
                    .await;
                Err(LegacyDeveloperAtomicErrorV1::Unavailable)
            }
        }
    }
}

fn map_secret_error(error: LegacyDeveloperSecretErrorV1) -> LegacyDeveloperAtomicErrorV1 {
    match error {
        LegacyDeveloperSecretErrorV1::Unavailable => {
            LegacyDeveloperAtomicErrorV1::SecretUnavailable
        }
        LegacyDeveloperSecretErrorV1::InvalidMaterial => LegacyDeveloperAtomicErrorV1::Corrupt,
    }
}

#[async_trait]
impl LegacyDeveloperAtomicPortV1 for D1LegacyDeveloperAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyDeveloperCommandV1,
        browser_fence: &LegacyDeveloperBrowserFenceV1,
        secrets: &dyn LegacyDeveloperSecretAuthorityV1,
    ) -> AtomicResult<LegacyDeveloperAtomicOutcomeV1> {
        let fence = *browser_fence;
        let action = Action::from_command(command);
        let request_digest = lower_hex(command.fence().request_fingerprint());
        let command_actor = command.fence().authority().actor_id().to_string();
        if fence.actor_id().to_string() != command_actor {
            let _ = self
                .consume_only(fence, action, None, &request_digest, "rejected")
                .await;
            return Err(LegacyDeveloperAtomicErrorV1::NotOwner);
        }
        let key_digest = operation_key_digest(
            &command_actor,
            action,
            command.fence().idempotency_key().expose(),
        );

        match self.operation(&command_actor, action, &key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                return self
                    .existing_outcome(command, fence, &operation, &request_digest)
                    .await;
            }
            Ok(Some(operation)) => {
                if operation.validate_identity().is_err() {
                    let _ = self
                        .consume_only(fence, action, None, &request_digest, "rejected")
                        .await;
                    return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
                }
                return self
                    .reject_and_return(
                        fence,
                        action,
                        Some(&operation.operation_id),
                        &request_digest,
                        "conflict",
                        LegacyDeveloperAtomicErrorV1::Conflict,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self
                    .consume_only(fence, action, None, &request_digest, "rejected")
                    .await;
                return Err(error);
            }
        }

        match self
            .execute_fresh(command, fence, &key_digest, &request_digest, secrets)
            .await
        {
            Ok(receipt) => Ok(LegacyDeveloperAtomicOutcomeV1::Applied(receipt)),
            Err(error) => {
                self.reconcile(
                    command,
                    fence,
                    &command_actor,
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

#[cfg(test)]
mod tests {
    use super::*;
    use frame_application::{
        LegacyDeveloperAdapterV1, LegacyDeveloperCredentialV1, LegacyDeveloperInputV1,
        LegacyDeveloperRequestV1,
    };

    fn actor() -> UserId {
        UserId::parse("00000000-0000-7000-8000-000000000001").expect("fixture actor")
    }

    fn create_command() -> LegacyDeveloperCommandV1 {
        LegacyDeveloperAdapterV1::create_app()
            .prepare(&LegacyDeveloperRequestV1 {
                credential: Some(LegacyDeveloperCredentialV1::Session),
                actor_id: Some(actor()),
                idempotency_key: Some("developer-runtime-test-0001".into()),
                input: LegacyDeveloperInputV1::CreateApp {
                    name: "Runtime test".into(),
                    environment: LegacyDeveloperEnvironmentV1::Development,
                },
            })
            .expect("create command")
    }

    fn authority() -> LocalLegacyDeveloperSecretAuthorityV1 {
        LocalLegacyDeveloperSecretAuthorityV1::from_hex(&"11".repeat(32)).expect("local authority")
    }

    #[test]
    fn local_key_material_is_strict_and_debug_redacted() {
        for invalid in [
            "",
            "11",
            &"00".repeat(32),
            &"AA".repeat(32),
            &format!("{}g", "1".repeat(63)),
        ] {
            assert_eq!(
                LocalLegacyDeveloperSecretAuthorityV1::from_hex(invalid).err(),
                Some(LegacyDeveloperSecretErrorV1::Unavailable)
            );
        }
        let authority = authority();
        let debug = format!("{authority:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(&"11".repeat(16)));
    }

    #[test]
    fn protected_generation_round_trips_only_through_bound_replay() {
        let authority = authority();
        let command = create_command();
        let context = command
            .secret_generation_context()
            .expect("generation context");
        let provisioning = futures::executor::block_on(authority.generate_protected(&context))
            .expect("protected provisioning");
        let keys = provisioning.keys();
        let revealed = futures::executor::block_on(authority.reveal(keys.replay()))
            .expect("authenticated reveal");
        assert!(revealed.expose_public_key().starts_with("cpk_"));
        assert!(revealed.expose_secret_key().starts_with("csk_"));
        assert_eq!(revealed.expose_public_key().len(), 34);
        assert_eq!(revealed.expose_secret_key().len(), 34);
        assert_eq!(keys.public_key().key_prefix().len(), 12);
        assert_eq!(keys.secret_key().key_prefix().len(), 12);
        assert_eq!(keys.replay().binding(), &context.replay_binding());
        for protected in [
            keys.public_key().encrypted_key().expose_for_persistence(),
            keys.secret_key().encrypted_key().expose_for_persistence(),
            keys.replay().ciphertext().expose_for_persistence(),
        ] {
            assert!(!protected.contains(revealed.expose_public_key()));
            assert!(!protected.contains(revealed.expose_secret_key()));
        }
        let public_plaintext = authority
            .open(
                b"key-at-rest-public",
                keys.replay().binding(),
                keys.public_key().encrypted_key().expose_for_persistence(),
            )
            .expect("at-rest public decrypt");
        assert_eq!(public_plaintext, revealed.expose_public_key().as_bytes());
    }

    #[test]
    fn replay_tamper_and_binding_substitution_fail_closed() {
        let authority = authority();
        let command = create_command();
        let context = command
            .secret_generation_context()
            .expect("generation context");
        let provisioning = futures::executor::block_on(authority.generate_protected(&context))
            .expect("protected provisioning");
        let replay = provisioning.keys().replay();
        let mut tampered = replay.ciphertext().expose_for_persistence().to_owned();
        let replacement = if tampered.ends_with('A') { 'B' } else { 'A' };
        tampered.pop();
        tampered.push(replacement);
        let tampered = LegacyDeveloperSealedKeyReplayV1::new(
            LegacyDeveloperProtectedBlobV1::new(tampered).expect("tampered blob"),
            *replay.binding(),
        );
        assert_eq!(
            futures::executor::block_on(authority.reveal(&tampered)).err(),
            Some(LegacyDeveloperSecretErrorV1::InvalidMaterial)
        );
        let wrong_binding =
            LegacyDeveloperSealedKeyReplayV1::new(replay.ciphertext().clone(), [7; 32]);
        assert_eq!(
            futures::executor::block_on(authority.reveal(&wrong_binding)).err(),
            Some(LegacyDeveloperSecretErrorV1::InvalidMaterial)
        );
    }

    #[test]
    fn encoding_hmac_and_cap_nanoid_helpers_are_canonical() {
        for length in 1..=65 {
            let bytes = (0..length).map(|value| value as u8).collect::<Vec<_>>();
            let encoded = base64_url_encode(&bytes);
            assert_eq!(base64_url_decode(&encoded), Some(bytes));
            assert!(!encoded.contains('='));
        }
        assert_eq!(base64_url_decode("A"), None);
        assert_eq!(base64_url_decode("AA="), None);
        assert_eq!(
            lower_hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        for _ in 0..64 {
            let value = random_cap_nanoid(15).expect("Cap NanoID");
            assert_eq!(value.len(), 15);
            assert!(value.bytes().all(|byte| CAP_ALPHABET.contains(&byte)));
        }
    }

    #[test]
    fn operation_keys_are_actor_action_and_secret_scoped() {
        let actor = actor().to_string();
        let base = operation_key_digest(&actor, Action::CreateApp, "browser-key-0001");
        assert_eq!(base.len(), 64);
        assert!(!base.contains("browser-key-0001"));
        assert_ne!(
            base,
            operation_key_digest(&actor, Action::UpdateApp, "browser-key-0001")
        );
        assert_ne!(
            base,
            operation_key_digest(
                "00000000-0000-7000-8000-000000000002",
                Action::CreateApp,
                "browser-key-0001"
            )
        );
        assert_ne!(
            base,
            operation_key_digest(&actor, Action::CreateApp, "browser-key-0002")
        );
    }

    #[test]
    fn d1_sentinels_map_to_fail_closed_typed_errors() {
        assert_eq!(
            map_d1_message(AUTHORITY_SENTINEL),
            LegacyDeveloperAtomicErrorV1::StaleAuthority
        );
        assert_eq!(
            map_d1_message(DOMAIN_UNIQUE_SENTINEL),
            LegacyDeveloperAtomicErrorV1::DuplicateDomain
        );
        assert_eq!(
            map_d1_message(OPERATION_UNIQUE_SENTINEL),
            LegacyDeveloperAtomicErrorV1::Conflict
        );
        assert_eq!(
            map_d1_message(CORRUPT_SENTINEL),
            LegacyDeveloperAtomicErrorV1::Corrupt
        );
        assert_eq!(
            map_d1_message("transport"),
            LegacyDeveloperAtomicErrorV1::Unavailable
        );
    }

    #[test]
    fn sql_surface_keeps_authority_targets_and_effects_exact() {
        for token in [
            "operation.actor_id = ?1",
            "operation.action = ?2",
            "operation.idempotency_key_digest = ?3",
            "LIMIT 2",
        ] {
            assert!(OPERATION_BY_KEY_SQL.contains(token));
        }
        for token in [
            "app.id = ?1",
            "app.owner_id = ?2",
            "app.deleted_at_ms IS NULL",
            "LIMIT 2",
        ] {
            assert!(APP_AUTHORITY_SNAPSHOT_SQL.contains(token));
        }
        assert!(UPDATE_APP_SQL.contains("AND ?13 = 1"));
        assert!(DOMAIN_DELETE_SQL.contains("WHERE id = ?2 AND app_id = ?3"));
        assert!(VIDEO_DELETE_SQL.contains("WHERE id = ?2 AND app_id = ?3"));
        assert!(BROWSER_GRANT_DELETE_SQL.contains("RETURNING id AS mutation_grant_id"));
        assert!(DURABLE_RECEIPT_ASSERT_SQL.contains("proof.outcome = ?8"));
        assert!(!Action::CreateApp.revalidate());
        assert!(Action::UpdateAutoTopUp.revalidate());
    }
}
