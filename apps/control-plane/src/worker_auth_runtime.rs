//! Cloudflare Worker authentication issuance and delivery boundary.
//!
//! This module owns only browser authentication transport. Durable identity,
//! verification, rate-limit, audit, and session decisions remain in
//! `AuthService` and `D1AuthStateRepository`. Render never sees these request
//! bodies or cookies because Cloudflare routes the exact `/api/v1/web/auth/*`
//! paths to this Worker.

use std::{collections::HashMap, fmt};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, Tag, aead::AeadInPlace};
use frame_application::{
    AbuseContext, AuthFailure, AuthService, IssuedSession, VerificationConsumeOutcome,
    VerificationIssueReceipt,
};
use frame_domain::{
    AbuseSignal, ApiKeySecret, AuthClientKind, CorrelationId, CsrfToken, DeliveryDestinationRef,
    DurationMillis, OAuthState, OneTimeCode, OpaqueAuthToken, PkceVerifier, SealedDeliveryEnvelope,
    TimestampMillis, UserId, VerificationChannel, VerificationPurpose, VerificationSecret,
};
use frame_ports::{
    AuthDeliveryAcknowledgeOutcome, AuthDeliverySealer, AuthSecretSource, AuthStateRepository,
    Clock, PortError, VerificationDeliveryMaterial,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{Env, Error, Request, Response, Result, send::IntoSendFuture};
use zeroize::Zeroize;

use crate::{
    auth_repository::D1AuthStateRepository,
    browser_web_runtime::{
        self, BrowserWebFailure, BrowserWebOutcome, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME,
    },
};

pub const PENDING_COOKIE_NAME: &str = "__Host-frame_auth_pending";

const DELIVERY_KEYRING_SECRET: &str = "FRAME_AUTH_DELIVERY_KEYRING_V1";
const PENDING_KEYRING_SECRET: &str = "FRAME_AUTH_PENDING_KEYRING_V1";
const AUTH_FORM_MAX_BYTES: usize = 2_048;
const PENDING_TTL_MS: i64 = 15 * 60 * 1_000;
const DELIVERY_PLAINTEXT_BYTES: usize = 1_024;
const PENDING_PLAINTEXT_BYTES: usize = 1_024;
const AEAD_NONCE_BYTES: usize = 12;
const AEAD_TAG_BYTES: usize = 16;
const DELIVERY_HEADER_BYTES: usize = 2 + AEAD_NONCE_BYTES + 1 + 8 + 8;
const PENDING_HEADER_BYTES: usize = 2 + AEAD_NONCE_BYTES + 8;
const DELIVERY_PAYLOAD_BYTES: usize =
    DELIVERY_HEADER_BYTES + DELIVERY_PLAINTEXT_BYTES + AEAD_TAG_BYTES;
const PENDING_PAYLOAD_BYTES: usize =
    PENDING_HEADER_BYTES + PENDING_PLAINTEXT_BYTES + AEAD_TAG_BYTES;
// Signup, login, and recovery retain distinct audit actions but deliberately
// share one rate-limit bucket. The per-tick dispatcher has admission headroom
// while reserving a conservative portion of Cloudflare's 1,000-query Worker
// invocation limit for the other scheduled jobs. The bound intentionally
// exceeds the observed 37-statement success path so error handling, fenced
// repository assertions, and handoff verification retain headroom.
#[cfg(test)]
const BROWSER_AUTH_DELIVERY_ACTION_CLASSES: usize = 1;
const AUTH_DELIVERY_D1_STATEMENTS_PER_ITEM: usize = 48;
const SCHEDULED_D1_STATEMENT_LIMIT: usize = 1_000;
const SCHEDULED_D1_STATEMENT_RESERVE: usize = 400;
const AUTH_DELIVERY_DISPATCH_BUDGET_PER_TICK: usize = (SCHEDULED_D1_STATEMENT_LIMIT
    - SCHEDULED_D1_STATEMENT_RESERVE)
    / AUTH_DELIVERY_D1_STATEMENTS_PER_ITEM;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserAuthStart {
    Login,
    Signup,
    Recovery,
}

impl BrowserAuthStart {
    const fn journey(self) -> PendingJourney {
        match self {
            Self::Login => PendingJourney::SignIn,
            Self::Signup => PendingJourney::Signup,
            Self::Recovery => PendingJourney::Recovery,
        }
    }

    const fn purpose(self) -> VerificationPurpose {
        match self {
            Self::Login => VerificationPurpose::SignIn,
            Self::Signup => VerificationPurpose::IdentityProvisioning,
            Self::Recovery => VerificationPurpose::AccountRecovery,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FixedClock(TimestampMillis);

impl Clock for FixedClock {
    fn now(&self) -> std::result::Result<TimestampMillis, PortError> {
        Ok(self.0)
    }
}

#[derive(Clone)]
struct AeadKey {
    version: u16,
    material: [u8; 32],
}

impl Drop for AeadKey {
    fn drop(&mut self) {
        self.material.zeroize();
    }
}

#[derive(Clone)]
struct AeadKeyRing {
    active: AeadKey,
    fallback: Vec<AeadKey>,
}

impl AeadKeyRing {
    fn parse(value: &str) -> BrowserWebOutcome<Self> {
        if value.len() > 4_096 {
            return Err(BrowserWebFailure::Unavailable);
        }
        let wire = serde_json::from_str::<AeadKeyRingWire>(value)
            .map_err(|_| BrowserWebFailure::Unavailable)?;
        let active = decode_aead_key(wire.active)?;
        let fallback = wire
            .fallback
            .into_iter()
            .map(decode_aead_key)
            .collect::<BrowserWebOutcome<Vec<_>>>()?;
        if fallback.len() > 4
            || fallback.iter().any(|key| key.version == active.version)
            || fallback.iter().enumerate().any(|(index, key)| {
                fallback[..index]
                    .iter()
                    .any(|other| other.version == key.version)
            })
        {
            return Err(BrowserWebFailure::Unavailable);
        }
        Ok(Self { active, fallback })
    }

    fn from_env(env: &Env, name: &str) -> BrowserWebOutcome<Self> {
        let secret = env
            .secret(name)
            .map_err(|_| BrowserWebFailure::Unavailable)?;
        let mut encoded = secret.to_string();
        let parsed = Self::parse(&encoded);
        encoded.zeroize();
        parsed
    }

    fn by_version(&self, version: u16) -> Option<&AeadKey> {
        std::iter::once(&self.active)
            .chain(self.fallback.iter())
            .find(|key| key.version == version)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AeadKeyRingWire {
    active: AeadKeyWire,
    #[serde(default)]
    fallback: Vec<AeadKeyWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AeadKeyWire {
    version: u16,
    material_hex: String,
}

impl Drop for AeadKeyWire {
    fn drop(&mut self) {
        self.material_hex.zeroize();
    }
}

fn decode_aead_key(wire: AeadKeyWire) -> BrowserWebOutcome<AeadKey> {
    if wire.version == 0
        || wire.material_hex.len() != 64
        || !wire
            .material_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BrowserWebFailure::Unavailable);
    }
    let mut material = [0_u8; 32];
    for (index, pair) in wire.material_hex.as_bytes().chunks_exact(2).enumerate() {
        let (Some(high), Some(low)) = (hex_nibble(pair[0]), hex_nibble(pair[1])) else {
            material.zeroize();
            return Err(BrowserWebFailure::Unavailable);
        };
        material[index] = (high << 4) | low;
    }
    if material.iter().all(|byte| *byte == 0) {
        return Err(BrowserWebFailure::Unavailable);
    }
    Ok(AeadKey {
        version: wire.version,
        material,
    })
}

#[derive(Debug, Default, Clone, Copy)]
struct WorkerAuthSecretSource;

impl AuthSecretSource for WorkerAuthSecretSource {
    fn session_token(&self) -> std::result::Result<OpaqueAuthToken, PortError> {
        OpaqueAuthToken::parse(random_url_secret(32)?)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn csrf_token(&self) -> std::result::Result<CsrfToken, PortError> {
        CsrfToken::parse(random_url_secret(32)?)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn api_key(&self) -> std::result::Result<ApiKeySecret, PortError> {
        ApiKeySecret::parse(format!("frm_{}", random_url_secret(32)?))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn oauth_state(&self) -> std::result::Result<OAuthState, PortError> {
        OAuthState::parse(random_url_secret(32)?)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn pkce_verifier(&self) -> std::result::Result<PkceVerifier, PortError> {
        PkceVerifier::parse(random_url_secret(32)?)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn verification_secret(
        &self,
        channel: VerificationChannel,
    ) -> std::result::Result<VerificationSecret, PortError> {
        match channel {
            VerificationChannel::MagicLink => OpaqueAuthToken::parse(random_url_secret(32)?)
                .map(VerificationSecret::MagicLink)
                .map_err(|error| PortError::Adapter(error.to_string())),
            VerificationChannel::OneTimeCode => OneTimeCode::parse(random_one_time_code()?)
                .map(VerificationSecret::OneTimeCode)
                .map_err(|error| PortError::Adapter(error.to_string())),
        }
    }
}

fn random_bytes<const N: usize>() -> std::result::Result<[u8; N], PortError> {
    let mut value = [0_u8; N];
    getrandom::fill(&mut value)
        .map_err(|_| PortError::Adapter("Worker CSPRNG is unavailable".into()))?;
    Ok(value)
}

fn random_url_secret(bytes: usize) -> std::result::Result<String, PortError> {
    if bytes != 32 {
        return Err(PortError::InvalidRequest(
            "unsupported authentication secret size".into(),
        ));
    }
    Ok(base64_url_encode(&random_bytes::<32>()?))
}

fn random_one_time_code() -> std::result::Result<String, PortError> {
    const SPACE: u32 = 1_000_000;
    const LIMIT: u32 = u32::MAX - (u32::MAX % SPACE);
    loop {
        let candidate = u32::from_be_bytes(random_bytes::<4>()?);
        if candidate < LIMIT {
            return Ok(format!("{:06}", candidate % SPACE));
        }
    }
}

#[derive(Clone)]
struct WorkerDeliverySealer {
    key: AeadKey,
}

impl WorkerDeliverySealer {
    fn from_env(env: &Env) -> BrowserWebOutcome<Self> {
        Ok(Self {
            key: AeadKeyRing::from_env(env, DELIVERY_KEYRING_SECRET)?.active,
        })
    }
}

impl AuthDeliverySealer for WorkerDeliverySealer {
    fn seal(
        &self,
        material: &VerificationDeliveryMaterial,
        now: TimestampMillis,
    ) -> std::result::Result<SealedDeliveryEnvelope, PortError> {
        let destination = material.destination.expose_for_sealing();
        let secret = material.secret.expose_for_hashing();
        let destination_len = u16::try_from(destination.len())
            .map_err(|_| PortError::InvalidRequest("delivery destination is too large".into()))?;
        let secret_len = u16::try_from(secret.len())
            .map_err(|_| PortError::InvalidRequest("delivery secret is too large".into()))?;
        if 5 + destination.len() + secret.len() > DELIVERY_PLAINTEXT_BYTES {
            return Err(PortError::InvalidRequest(
                "delivery material exceeds fixed envelope".into(),
            ));
        }

        let mut plaintext = random_bytes::<DELIVERY_PLAINTEXT_BYTES>()?;
        plaintext[..2].copy_from_slice(&destination_len.to_be_bytes());
        plaintext[2..4].copy_from_slice(&secret_len.to_be_bytes());
        plaintext[4] = verification_channel_code(material.secret.channel());
        let destination_end = 5 + destination.len();
        plaintext[5..destination_end].copy_from_slice(destination);
        plaintext[destination_end..destination_end + secret.len()].copy_from_slice(secret);

        let nonce = random_bytes::<AEAD_NONCE_BYTES>()?;
        let mut header = Vec::with_capacity(DELIVERY_HEADER_BYTES);
        header.extend_from_slice(&self.key.version.to_be_bytes());
        header.extend_from_slice(&nonce);
        header.push(verification_purpose_code(material.purpose));
        header.extend_from_slice(&material.expires_at.get().to_be_bytes());
        header.extend_from_slice(&now.get().to_be_bytes());
        let aad = delivery_aad(&header);
        let mut ciphertext = plaintext.to_vec();
        plaintext.zeroize();
        seal_in_place(&self.key, nonce, &aad, &mut ciphertext)?;

        let mut payload = header;
        payload.extend_from_slice(&ciphertext);
        ciphertext.zeroize();
        if payload.len() != DELIVERY_PAYLOAD_BYTES {
            payload.zeroize();
            return Err(PortError::Adapter(
                "fixed delivery envelope construction failed".into(),
            ));
        }
        SealedDeliveryEnvelope::new(payload, now)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }
}

fn seal_in_place(
    key: &AeadKey,
    nonce: [u8; AEAD_NONCE_BYTES],
    aad: &[u8],
    value: &mut Vec<u8>,
) -> std::result::Result<(), PortError> {
    let mut material = key.material;
    let cipher = Aes256Gcm::new_from_slice(&material);
    material.zeroize();
    let cipher = cipher
        .map_err(|_| PortError::Adapter("authentication encryption key is invalid".into()))?;
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), aad, value.as_mut_slice())
        .map_err(|_| PortError::Adapter("authentication encryption failed".into()))?;
    value.extend_from_slice(&tag);
    Ok(())
}

fn open_in_place(
    key: &AeadKey,
    nonce: [u8; AEAD_NONCE_BYTES],
    aad: &[u8],
    mut value: Vec<u8>,
) -> BrowserWebOutcome<Vec<u8>> {
    if value.len() < AEAD_TAG_BYTES {
        return Err(BrowserWebFailure::Invalid);
    }
    let mut material = key.material;
    let cipher = Aes256Gcm::new_from_slice(&material);
    material.zeroize();
    let cipher = cipher.map_err(|_| BrowserWebFailure::Unavailable)?;
    let plaintext_len = value.len() - AEAD_TAG_BYTES;
    let tag = Tag::clone_from_slice(&value[plaintext_len..]);
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            aad,
            &mut value[..plaintext_len],
            &tag,
        )
        .map_err(|_| BrowserWebFailure::Invalid)?;
    value.truncate(plaintext_len);
    Ok(value)
}

fn delivery_aad(header: &[u8]) -> Vec<u8> {
    let mut aad = b"frame/auth/delivery/aes-256-gcm/v1\0".to_vec();
    aad.extend_from_slice(header);
    aad
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PendingJourney {
    SignIn,
    Signup,
    Recovery,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingAuthState {
    schema_version: u8,
    journey: PendingJourney,
    identifier: String,
    display_name: Option<String>,
    user_id: Option<String>,
    verify_correlation_id: String,
    provision_correlation_id: String,
    session_correlation_id: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

impl fmt::Debug for PendingAuthState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingAuthState")
            .field("journey", &self.journey)
            .field("identifier", &"<redacted>")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "<redacted>"),
            )
            .field("user_id", &self.user_id.as_ref().map(|_| "<redacted>"))
            .field("correlation_ids", &"<redacted>")
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

impl PendingAuthState {
    fn new(
        journey: PendingJourney,
        identifier: String,
        display_name: Option<String>,
        now: TimestampMillis,
    ) -> BrowserWebOutcome<Self> {
        let expires_at_ms = now
            .get()
            .checked_add(PENDING_TTL_MS)
            .ok_or(BrowserWebFailure::Unavailable)?;
        Ok(Self {
            schema_version: 1,
            journey,
            identifier,
            display_name,
            user_id: (journey == PendingJourney::Signup).then(|| UserId::new().to_string()),
            verify_correlation_id: CorrelationId::new().to_string(),
            provision_correlation_id: CorrelationId::new().to_string(),
            session_correlation_id: CorrelationId::new().to_string(),
            issued_at_ms: now.get(),
            expires_at_ms,
        })
    }

    fn validate(&self, now: TimestampMillis) -> BrowserWebOutcome<()> {
        let valid_user_binding = match (self.journey, self.user_id.as_deref()) {
            (PendingJourney::Signup, Some(value)) => UserId::parse(value).is_ok(),
            (PendingJourney::SignIn | PendingJourney::Recovery, None) => true,
            _ => false,
        };
        if self.schema_version != 1
            || normalize_identifier(&self.identifier).as_deref() != Ok(self.identifier.as_str())
            || self.issued_at_ms < 0
            || self.expires_at_ms.checked_sub(self.issued_at_ms) != Some(PENDING_TTL_MS)
            || now.get() < self.issued_at_ms
            || now.get() >= self.expires_at_ms
            || CorrelationId::parse(&self.verify_correlation_id).is_err()
            || CorrelationId::parse(&self.provision_correlation_id).is_err()
            || CorrelationId::parse(&self.session_correlation_id).is_err()
            || !valid_user_binding
            || (self.journey == PendingJourney::Signup) != self.display_name.is_some()
            || self
                .display_name
                .as_deref()
                .is_some_and(|value| validate_display_name(value).is_err())
        {
            return Err(BrowserWebFailure::Invalid);
        }
        Ok(())
    }

    const fn purpose(&self) -> VerificationPurpose {
        match self.journey {
            PendingJourney::SignIn => VerificationPurpose::SignIn,
            PendingJourney::Signup => VerificationPurpose::IdentityProvisioning,
            PendingJourney::Recovery => VerificationPurpose::AccountRecovery,
        }
    }
}

struct PendingCookieCipher {
    keys: AeadKeyRing,
}

impl PendingCookieCipher {
    fn from_env(env: &Env) -> BrowserWebOutcome<Self> {
        Ok(Self {
            keys: AeadKeyRing::from_env(env, PENDING_KEYRING_SECRET)?,
        })
    }

    fn seal(&self, state: &PendingAuthState) -> BrowserWebOutcome<String> {
        let mut encoded = serde_json::to_vec(state).map_err(|_| BrowserWebFailure::Unavailable)?;
        if encoded.len() + 2 > PENDING_PLAINTEXT_BYTES {
            encoded.zeroize();
            return Err(BrowserWebFailure::Unavailable);
        }
        let encoded_len = match u16::try_from(encoded.len()) {
            Ok(length) => length,
            Err(_) => {
                encoded.zeroize();
                return Err(BrowserWebFailure::Unavailable);
            }
        };
        let mut plaintext = random_bytes::<PENDING_PLAINTEXT_BYTES>()
            .map_err(|_| BrowserWebFailure::Unavailable)?;
        plaintext[..2].copy_from_slice(&encoded_len.to_be_bytes());
        plaintext[2..2 + encoded.len()].copy_from_slice(&encoded);
        encoded.zeroize();
        let nonce =
            random_bytes::<AEAD_NONCE_BYTES>().map_err(|_| BrowserWebFailure::Unavailable)?;
        let mut header = Vec::with_capacity(PENDING_HEADER_BYTES);
        header.extend_from_slice(&self.keys.active.version.to_be_bytes());
        header.extend_from_slice(&nonce);
        header.extend_from_slice(&state.expires_at_ms.to_be_bytes());
        let aad = pending_aad(&header);
        let mut ciphertext = plaintext.to_vec();
        plaintext.zeroize();
        seal_in_place(&self.keys.active, nonce, &aad, &mut ciphertext)
            .map_err(|_| BrowserWebFailure::Unavailable)?;
        let mut payload = header;
        payload.extend_from_slice(&ciphertext);
        ciphertext.zeroize();
        if payload.len() != PENDING_PAYLOAD_BYTES {
            payload.zeroize();
            return Err(BrowserWebFailure::Unavailable);
        }
        let value = base64_url_encode(&payload);
        payload.zeroize();
        Ok(value)
    }

    fn open(&self, value: &str, now: TimestampMillis) -> BrowserWebOutcome<PendingAuthState> {
        let mut payload = base64_url_decode(value).ok_or(BrowserWebFailure::Invalid)?;
        if payload.len() != PENDING_PAYLOAD_BYTES {
            payload.zeroize();
            return Err(BrowserWebFailure::Invalid);
        }
        let version = u16::from_be_bytes([payload[0], payload[1]]);
        let nonce: [u8; AEAD_NONCE_BYTES] = payload[2..2 + AEAD_NONCE_BYTES]
            .try_into()
            .map_err(|_| BrowserWebFailure::Invalid)?;
        let expiry_offset = 2 + AEAD_NONCE_BYTES;
        let expires_at_ms = i64::from_be_bytes(
            payload[expiry_offset..expiry_offset + 8]
                .try_into()
                .map_err(|_| BrowserWebFailure::Invalid)?,
        );
        if now.get() >= expires_at_ms {
            payload.zeroize();
            return Err(BrowserWebFailure::Unauthenticated);
        }
        let header = payload[..PENDING_HEADER_BYTES].to_vec();
        let aad = pending_aad(&header);
        let key = self
            .keys
            .by_version(version)
            .ok_or(BrowserWebFailure::Invalid)?;
        let ciphertext = payload.split_off(PENDING_HEADER_BYTES);
        payload.zeroize();
        let mut plaintext = open_in_place(key, nonce, &aad, ciphertext)?;
        if plaintext.len() != PENDING_PLAINTEXT_BYTES {
            plaintext.zeroize();
            return Err(BrowserWebFailure::Invalid);
        }
        let encoded_len = usize::from(u16::from_be_bytes([plaintext[0], plaintext[1]]));
        if encoded_len == 0 || encoded_len + 2 > plaintext.len() {
            plaintext.zeroize();
            return Err(BrowserWebFailure::Invalid);
        }
        let state = match serde_json::from_slice::<PendingAuthState>(&plaintext[2..2 + encoded_len])
        {
            Ok(state) => state,
            Err(_) => {
                plaintext.zeroize();
                return Err(BrowserWebFailure::Invalid);
            }
        };
        plaintext.zeroize();
        if state.expires_at_ms != expires_at_ms {
            return Err(BrowserWebFailure::Invalid);
        }
        state.validate(now)?;
        Ok(state)
    }
}

fn pending_aad(header: &[u8]) -> Vec<u8> {
    let mut aad = b"frame/auth/pending-cookie/aes-256-gcm/v1\0".to_vec();
    aad.extend_from_slice(header);
    aad
}

pub async fn start(
    request: &mut Request,
    env: &Env,
    action: BrowserAuthStart,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Response>> {
    if let Err(failure) = validate_auth_post(request) {
        return Ok(Err(failure));
    }
    let fields = match decode_form(request, action == BrowserAuthStart::Signup).await? {
        Ok(fields) => fields,
        Err(failure) => return Ok(Err(failure)),
    };
    let identifier = match fields
        .get("email")
        .ok_or(BrowserWebFailure::Invalid)
        .and_then(|value| normalize_identifier(value))
    {
        Ok(identifier) => identifier,
        Err(failure) => return Ok(Err(failure)),
    };
    let display_name = if action == BrowserAuthStart::Signup {
        match fields
            .get("display_name")
            .ok_or(BrowserWebFailure::Invalid)
            .and_then(|value| validate_display_name(value))
        {
            Ok(value) => Some(value),
            Err(failure) => return Ok(Err(failure)),
        }
    } else {
        None
    };
    let now = TimestampMillis::new(now_ms)
        .map_err(|_| Error::RustError("authentication clock is invalid".into()))?;
    let pending = match PendingAuthState::new(action.journey(), identifier, display_name, now) {
        Ok(pending) => pending,
        Err(failure) => return Ok(Err(failure)),
    };
    let source = match abuse_source(request) {
        Ok(source) => source,
        Err(failure) => return Ok(Err(failure)),
    };
    let device = match abuse_device(request) {
        Ok(device) => device,
        Err(failure) => return Ok(Err(failure)),
    };
    let database = env.d1("DB")?;
    let repository = D1AuthStateRepository::new(&database);
    let clock = FixedClock(now);
    let secrets = WorkerAuthSecretSource;
    let sealer = match WorkerDeliverySealer::from_env(env) {
        Ok(sealer) => sealer,
        Err(failure) => return Ok(Err(failure)),
    };
    let hash_keys = match browser_web_runtime::auth_hash_keyring(env) {
        Ok(keys) => keys,
        Err(failure) => return Ok(Err(failure)),
    };
    let policy = match browser_web_runtime::verifier_policy() {
        Ok(policy) => policy,
        Err(failure) => return Ok(Err(failure)),
    };
    let service = AuthService::new(&repository, &clock, &secrets, &sealer, hash_keys, policy);
    let abuse = AbuseContext {
        source: &source,
        device: &device,
    };
    let correlation = CorrelationId::parse(&pending.verify_correlation_id)
        .map_err(|_| Error::RustError("pending authentication correlation is invalid".into()))?;
    let issue = match action {
        BrowserAuthStart::Signup => {
            let user_id = pending
                .user_id
                .as_deref()
                .and_then(|value| UserId::parse(value).ok())
                .ok_or_else(|| Error::RustError("pending signup authority is invalid".into()))?;
            service
                .issue_identity_provisioning_verification(
                    &pending.identifier,
                    user_id,
                    1,
                    VerificationChannel::OneTimeCode,
                    abuse,
                    correlation,
                )
                .await
        }
        BrowserAuthStart::Login | BrowserAuthStart::Recovery => {
            service
                .issue_verification(
                    &pending.identifier,
                    action.purpose(),
                    VerificationChannel::OneTimeCode,
                    abuse,
                    correlation,
                )
                .await
        }
    };
    match issue {
        Ok(VerificationIssueReceipt::Accepted) => {}
        Ok(VerificationIssueReceipt::RateLimited { .. }) | Err(AuthFailure::RateLimited) => {
            return Ok(Err(BrowserWebFailure::Forbidden));
        }
        Err(failure) => return Ok(Err(map_auth_failure(failure))),
    }
    let pending_cookie =
        match PendingCookieCipher::from_env(env).and_then(|cipher| cipher.seal(&pending)) {
            Ok(cookie) => cookie,
            Err(failure) => return Ok(Err(failure)),
        };
    Ok(Ok(redirect_response(
        "/verify",
        &[pending_cookie_header(&pending_cookie)],
    )?))
}

pub async fn verify(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Response>> {
    if let Err(failure) = validate_auth_post(request) {
        return Ok(Err(failure));
    }
    let fields = match decode_form(request, false).await? {
        Ok(fields) => fields,
        Err(failure) => return Ok(Err(failure)),
    };
    if fields.len() != 1 || !fields.contains_key("otp") {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let Some(pending_cookie) = unique_cookie(request, PENDING_COOKIE_NAME, 2_048)? else {
        return Ok(Err(BrowserWebFailure::Unauthenticated));
    };
    let now = TimestampMillis::new(now_ms)
        .map_err(|_| Error::RustError("authentication clock is invalid".into()))?;
    let pending = match PendingCookieCipher::from_env(env)
        .and_then(|cipher| cipher.open(&pending_cookie, now))
    {
        Ok(pending) => pending,
        Err(failure) => return Ok(Err(failure)),
    };
    let secret = match fields
        .get("otp")
        .cloned()
        .ok_or(BrowserWebFailure::Invalid)
        .and_then(|value| {
            OneTimeCode::parse(value)
                .map(VerificationSecret::OneTimeCode)
                .map_err(|_| BrowserWebFailure::Invalid)
        }) {
        Ok(secret) => secret,
        Err(failure) => return Ok(Err(failure)),
    };
    let source = match abuse_source(request) {
        Ok(source) => source,
        Err(failure) => return Ok(Err(failure)),
    };
    let device = match abuse_device(request) {
        Ok(device) => device,
        Err(failure) => return Ok(Err(failure)),
    };
    let database = env.d1("DB")?;
    let repository = D1AuthStateRepository::new(&database);
    let clock = FixedClock(now);
    let secrets = WorkerAuthSecretSource;
    let sealer = match WorkerDeliverySealer::from_env(env) {
        Ok(sealer) => sealer,
        Err(failure) => return Ok(Err(failure)),
    };
    let hash_keys = match browser_web_runtime::auth_hash_keyring(env) {
        Ok(keys) => keys,
        Err(failure) => return Ok(Err(failure)),
    };
    let policy = match browser_web_runtime::verifier_policy() {
        Ok(policy) => policy,
        Err(failure) => return Ok(Err(failure)),
    };
    let service = AuthService::new(&repository, &clock, &secrets, &sealer, hash_keys, policy);
    let abuse = AbuseContext {
        source: &source,
        device: &device,
    };
    let correlation = CorrelationId::parse(&pending.verify_correlation_id)
        .map_err(|_| Error::RustError("pending authentication correlation is invalid".into()))?;
    let outcome = match service
        .consume_verification(
            &pending.identifier,
            pending.purpose(),
            &secret,
            abuse,
            correlation,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(failure) => return Ok(Err(map_auth_failure(failure))),
    };
    match outcome {
        VerificationConsumeOutcome::Verified(principal)
            if matches!(
                pending.journey,
                PendingJourney::SignIn | PendingJourney::Recovery
            ) =>
        {
            let correlation = CorrelationId::parse(&pending.session_correlation_id)
                .map_err(|_| Error::RustError("pending session correlation is invalid".into()))?;
            let session = match service
                .issue_session(principal, AuthClientKind::Browser, correlation)
                .await
            {
                Ok(session) => session,
                Err(failure) => return Ok(Err(map_auth_failure(failure))),
            };
            Ok(Ok(session_redirect(session)?))
        }
        VerificationConsumeOutcome::ProvisioningAuthorized(verified)
            if pending.journey == PendingJourney::Signup =>
        {
            if pending.user_id.as_deref() != Some(&verified.user_id().to_string()) {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            let destination = DeliveryDestinationRef::parse(format!(
                "verified:{}",
                hex_sha256(pending.identifier.as_bytes())
            ))
            .map_err(|_| Error::RustError("verified destination authority is invalid".into()))?;
            let correlation = CorrelationId::parse(&pending.provision_correlation_id)
                .map_err(|_| Error::RustError("pending provision correlation is invalid".into()))?;
            let user_id = match service
                .provision_identity(verified, destination, correlation)
                .await
            {
                Ok(user_id) => user_id,
                Err(failure) => return Ok(Err(map_auth_failure(failure))),
            };
            let display_name = pending.display_name.as_deref().unwrap_or_default();
            let updated = database
                .prepare(
                    "UPDATE users SET display_name=?2,updated_at_ms=?3 \
                     WHERE id=?1 AND status='active' AND deleted_at_ms IS NULL \
                       AND display_name IS NULL",
                )
                .bind(&[
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_str(display_name),
                    JsValue::from_f64(now.get() as f64),
                ])?
                .run()
                .into_send()
                .await?;
            if !updated.success() {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            #[derive(Deserialize)]
            struct DisplayNameRow {
                display_name: Option<String>,
            }
            let stored = database
                .prepare(
                    "SELECT display_name FROM users WHERE id=?1 AND status='active' \
                     AND deleted_at_ms IS NULL LIMIT 1",
                )
                .bind(&[JsValue::from_str(&user_id.to_string())])?
                .first::<DisplayNameRow>(None)
                .await?;
            if stored.and_then(|row| row.display_name).as_deref() != Some(display_name) {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            Ok(Ok(redirect_response(
                "/login",
                &[clear_cookie_header(PENDING_COOKIE_NAME, true)],
            )?))
        }
        VerificationConsumeOutcome::RateLimited { .. } => Ok(Err(BrowserWebFailure::Forbidden)),
        VerificationConsumeOutcome::Rejected
        | VerificationConsumeOutcome::Linked { .. }
        | VerificationConsumeOutcome::Verified(_)
        | VerificationConsumeOutcome::ProvisioningAuthorized(_) => {
            Ok(Err(BrowserWebFailure::Unauthenticated))
        }
    }
}

pub async fn logout(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Response>> {
    match browser_web_runtime::logout(request, env, now_ms).await? {
        Ok(()) => Ok(Ok(redirect_response(
            "/login",
            &[
                clear_cookie_header(SESSION_COOKIE_NAME, true),
                clear_cookie_header(CSRF_COOKIE_NAME, false),
                clear_cookie_header(PENDING_COOKIE_NAME, true),
            ],
        )?)),
        Err(failure) => Ok(Err(failure)),
    }
}

fn session_redirect(session: IssuedSession) -> Result<Response> {
    let token = std::str::from_utf8(session.token.expose_for_hashing())
        .map_err(|_| Error::RustError("session credential encoding is invalid".into()))?;
    let csrf = session
        .csrf_token
        .as_ref()
        .ok_or_else(|| Error::RustError("browser CSRF credential is unavailable".into()))?;
    let csrf = std::str::from_utf8(csrf.expose_for_hashing())
        .map_err(|_| Error::RustError("CSRF credential encoding is invalid".into()))?;
    let cookie = session
        .cookie
        .as_ref()
        .ok_or_else(|| Error::RustError("browser session cookie contract is unavailable".into()))?;
    let max_age = cookie.max_age().get() / 1_000;
    redirect_response(
        "/dashboard",
        &[
            format!(
                "{SESSION_COOKIE_NAME}={token}; Path=/; Max-Age={max_age}; Secure; HttpOnly; SameSite=Lax"
            ),
            format!(
                "{CSRF_COOKIE_NAME}={csrf}; Path=/; Max-Age={max_age}; Secure; SameSite=Strict"
            ),
            clear_cookie_header(PENDING_COOKIE_NAME, true),
        ],
    )
}

fn redirect_response(location: &str, cookies: &[String]) -> Result<Response> {
    let mut response = Response::empty()?.with_status(303);
    response.headers_mut().set("location", location)?;
    response.headers_mut().set("cache-control", "no-store")?;
    response.headers_mut().set("pragma", "no-cache")?;
    response
        .headers_mut()
        .set("referrer-policy", "no-referrer")?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    for cookie in cookies {
        response.headers_mut().append("set-cookie", cookie)?;
    }
    Ok(response)
}

fn pending_cookie_header(value: &str) -> String {
    format!(
        "{PENDING_COOKIE_NAME}={value}; Path=/; Max-Age={}; Secure; HttpOnly; SameSite=Strict",
        PENDING_TTL_MS / 1_000
    )
}

fn clear_cookie_header(name: &str, http_only: bool) -> String {
    format!(
        "{name}=; Path=/; Max-Age=0; Secure; {}SameSite=Strict",
        if http_only { "HttpOnly; " } else { "" }
    )
}

fn validate_auth_post(request: &Request) -> BrowserWebOutcome<()> {
    if request
        .headers()
        .get("authorization")
        .ok()
        .flatten()
        .is_some()
        || request
            .headers()
            .get("x-frame-tenant-id")
            .ok()
            .flatten()
            .is_some()
        || request
            .headers()
            .get("sec-fetch-site")
            .ok()
            .flatten()
            .as_deref()
            != Some("same-origin")
    {
        return Err(BrowserWebFailure::Forbidden);
    }
    let url = request.url().map_err(|_| BrowserWebFailure::Invalid)?;
    let origin = url.origin().ascii_serialization();
    if request.headers().get("origin").ok().flatten().as_deref() != Some(origin.as_str()) {
        return Err(BrowserWebFailure::Forbidden);
    }
    if request
        .headers()
        .get("content-encoding")
        .ok()
        .flatten()
        .is_some_and(|value| value != "identity")
        || request
            .headers()
            .get("content-type")
            .ok()
            .flatten()
            .is_none_or(|value| value != "application/x-www-form-urlencoded")
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(())
}

async fn decode_form(
    request: &mut Request,
    signup: bool,
) -> Result<BrowserWebOutcome<HashMap<String, String>>> {
    let declared = request
        .headers()
        .get("content-length")?
        .map(|value| value.parse::<usize>())
        .transpose()
        .ok()
        .flatten();
    if declared.is_some_and(|value| value == 0 || value > AUTH_FORM_MAX_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = request.bytes().await?;
    if bytes.is_empty()
        || bytes.len() > AUTH_FORM_MAX_BYTES
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let allowed: &[&str] = if signup {
        &["display_name", "email"]
    } else if bytes.starts_with(b"otp=") {
        &["otp"]
    } else {
        &["email"]
    };
    Ok(parse_form(&bytes, allowed))
}

fn parse_form(bytes: &[u8], allowed: &[&str]) -> BrowserWebOutcome<HashMap<String, String>> {
    let text = std::str::from_utf8(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
    let mut values = HashMap::new();
    for pair in text.split('&') {
        let (name, value) = pair.split_once('=').ok_or(BrowserWebFailure::Invalid)?;
        let name = percent_decode_form(name).ok_or(BrowserWebFailure::Invalid)?;
        let value = percent_decode_form(value).ok_or(BrowserWebFailure::Invalid)?;
        if !allowed.contains(&name.as_str())
            || values.insert(name, value).is_some()
            || values.len() > allowed.len()
        {
            return Err(BrowserWebFailure::Invalid);
        }
    }
    if values.len() != allowed.len() || allowed.iter().any(|field| !values.contains_key(*field)) {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(values)
}

fn percent_decode_form(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_nibble(bytes[index + 1])?;
                let low = hex_nibble(bytes[index + 2])?;
                output.push((high << 4) | low);
                index += 3;
            }
            b'%' => return None,
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).ok()
}

fn normalize_identifier(value: &str) -> BrowserWebOutcome<String> {
    if value.trim() != value
        || !(3..=254).contains(&value.len())
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(BrowserWebFailure::Invalid);
    }
    let normalized = value.to_ascii_lowercase();
    let Some((local, domain)) = normalized.split_once('@') else {
        return Err(BrowserWebFailure::Invalid);
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(normalized)
}

fn validate_display_name(value: &str) -> BrowserWebOutcome<String> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized.len() > 120
        || normalized.chars().any(char::is_control)
        || normalized.contains(['<', '>'])
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(normalized.to_owned())
}

fn abuse_source(request: &Request) -> BrowserWebOutcome<AbuseSignal> {
    let value = request
        .headers()
        .get("cf-connecting-ip")
        .map_err(|_| BrowserWebFailure::Invalid)?
        .unwrap_or_else(|| "unavailable".into());
    hashed_abuse_signal(b"source", value.as_bytes())
}

fn abuse_device(request: &Request) -> BrowserWebOutcome<AbuseSignal> {
    let value = request
        .headers()
        .get("user-agent")
        .map_err(|_| BrowserWebFailure::Invalid)?
        .unwrap_or_else(|| "unavailable".into());
    hashed_abuse_signal(b"device", value.as_bytes())
}

fn hashed_abuse_signal(domain: &[u8], value: &[u8]) -> BrowserWebOutcome<AbuseSignal> {
    let mut digest = Sha256::new();
    digest.update(b"frame/browser-auth-abuse/v1\0");
    digest.update((domain.len() as u32).to_be_bytes());
    digest.update(domain);
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
    AbuseSignal::parse(format!("v1:{:x}", digest.finalize()))
        .map_err(|_| BrowserWebFailure::Unavailable)
}

fn unique_cookie(request: &Request, name: &str, maximum: usize) -> Result<Option<String>> {
    let Some(header) = request.headers().get("cookie")? else {
        return Ok(None);
    };
    let mut found = None;
    for pair in header.split(';') {
        let Some((candidate, value)) = pair.trim().split_once('=') else {
            continue;
        };
        if candidate != name {
            continue;
        }
        if found.is_some() || value.is_empty() || value.len() > maximum {
            return Ok(None);
        }
        found = Some(value.to_owned());
    }
    Ok(found)
}

fn map_auth_failure(failure: AuthFailure) -> BrowserWebFailure {
    match failure {
        AuthFailure::Unauthenticated => BrowserWebFailure::Unauthenticated,
        AuthFailure::RequestRejected | AuthFailure::RateLimited => BrowserWebFailure::Forbidden,
        AuthFailure::InvalidRequest => BrowserWebFailure::Invalid,
        AuthFailure::Unavailable => BrowserWebFailure::Unavailable,
    }
}

#[derive(Debug, Deserialize)]
struct DeliveryHandoffRow {
    payload_sha256: String,
}

/// Drains a bounded batch of fenced ciphertext claims into the provider-neutral D1 handoff.
///
/// A future provider adapter consumes the handoff and performs the protected
/// email/SMS execution. The unique delivery ID makes a crash after insertion
/// but before outbox acknowledgement safely repeatable without duplicating the
/// provider handoff.
pub async fn dispatch_delivery_batch(env: Env) {
    for _ in 0..AUTH_DELIVERY_DISPATCH_BUDGET_PER_TICK {
        match dispatch_delivery_one_inner(&env).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(failure) => {
                worker::console_error!("authentication delivery dispatch failed class={failure}");
                if matches!(failure, "clock" | "lease" | "binding" | "claim" | "bind") {
                    return;
                }
                // Item-scoped shape/collision failures are already deferred,
                // and an acknowledged lease prevents this tick from selecting
                // the same item again. Continue so it cannot starve the queue.
            }
        }
    }
    worker::console_warn!("authentication delivery dispatch reached its bounded tick budget");
}

async fn dispatch_delivery_one_inner(env: &Env) -> std::result::Result<bool, &'static str> {
    let now_ms = js_sys::Date::now() as i64;
    let now = TimestampMillis::new(now_ms).map_err(|_| "clock")?;
    let lease = DurationMillis::new(60_000).map_err(|_| "lease")?;
    let database = env.d1("DB").map_err(|_| "binding")?;
    let repository = D1AuthStateRepository::new(&database);
    let Some(claim) = repository
        .claim_auth_delivery(now, lease)
        .await
        .map_err(|_| "claim")?
    else {
        return Ok(false);
    };
    let delivery_id = claim.delivery_id().to_string();
    let payload = claim.envelope().sealed_payload();
    if payload.len() != DELIVERY_PAYLOAD_BYTES {
        let retry_at = now
            .checked_add(DurationMillis::new(5 * 60_000).map_err(|_| "retry")?)
            .map_err(|_| "retry")?;
        let _ = repository.retry_auth_delivery(claim, now, retry_at).await;
        return Err("shape");
    }
    let payload_sha256 = hex_sha256(payload);
    let payload_hex = bytes_to_hex(payload);
    let inserted = database
        .prepare(
            "INSERT OR IGNORE INTO auth_delivery_provider_handoffs_v1( \
               delivery_id,payload_hex,payload_sha256,state,provider_attempt,created_at_ms,updated_at_ms) \
             VALUES (?1,?2,?3,'pending',0,?4,?4)",
        )
        .bind(&[
            JsValue::from_str(&delivery_id),
            JsValue::from_str(&payload_hex),
            JsValue::from_str(&payload_sha256),
            JsValue::from_f64(now_ms as f64),
        ])
        .map_err(|_| "bind")?
        .run()
        .into_send()
        .await
        .map_err(|_| "insert")?;
    if !inserted.success() {
        let retry_at = now
            .checked_add(DurationMillis::new(30_000).map_err(|_| "retry")?)
            .map_err(|_| "retry")?;
        let _ = repository.retry_auth_delivery(claim, now, retry_at).await;
        return Err("insert");
    }
    let stored = database
        .prepare(
            "SELECT payload_sha256 FROM auth_delivery_provider_handoffs_v1 WHERE delivery_id=?1",
        )
        .bind(&[JsValue::from_str(&delivery_id)])
        .map_err(|_| "bind")?
        .first::<DeliveryHandoffRow>(None)
        .await
        .map_err(|_| "read")?;
    if stored.as_ref().map(|row| row.payload_sha256.as_str()) != Some(payload_sha256.as_str()) {
        let retry_at = now
            .checked_add(DurationMillis::new(5 * 60_000).map_err(|_| "retry")?)
            .map_err(|_| "retry")?;
        let _ = repository.retry_auth_delivery(claim, now, retry_at).await;
        return Err("collision");
    }
    match repository
        .acknowledge_auth_delivery(claim, now)
        .await
        .map_err(|_| "ack")?
    {
        AuthDeliveryAcknowledgeOutcome::Acknowledged
        | AuthDeliveryAcknowledgeOutcome::StaleLease => Ok(true),
    }
}

fn verification_purpose_code(purpose: VerificationPurpose) -> u8 {
    match purpose {
        VerificationPurpose::IdentityProvisioning => 1,
        VerificationPurpose::EmailVerify => 2,
        VerificationPurpose::SignIn => 3,
        VerificationPurpose::AccountRecovery => 4,
        VerificationPurpose::AccountLink => 5,
    }
}

fn verification_channel_code(channel: VerificationChannel) -> u8 {
    match channel {
        VerificationChannel::MagicLink => 1,
        VerificationChannel::OneTimeCode => 2,
    }
}

fn base64_url_encode(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    let mut chunks = value.chunks_exact(3);
    for chunk in &mut chunks {
        output.push(char::from(ALPHABET[usize::from(chunk[0] >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from(((chunk[0] & 0x03) << 4) | (chunk[1] >> 4))],
        ));
        output.push(char::from(
            ALPHABET[usize::from(((chunk[1] & 0x0f) << 2) | (chunk[2] >> 6))],
        ));
        output.push(char::from(ALPHABET[usize::from(chunk[2] & 0x3f)]));
    }
    match chunks.remainder() {
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
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn bytes_to_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn hex_sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use frame_domain::{VerificationDestination, VerificationPurpose};

    use super::*;

    fn keyring() -> AeadKeyRing {
        AeadKeyRing::parse(&format!(
            r#"{{"active":{{"version":2,"material_hex":"{}"}},"fallback":[{{"version":1,"material_hex":"{}"}}]}}"#,
            "ab".repeat(32),
            "cd".repeat(32)
        ))
        .expect("valid test keyring")
    }

    #[test]
    fn worker_secret_source_uses_closed_shapes_and_fresh_material() {
        let source = WorkerAuthSecretSource;
        let mut tokens = HashSet::new();
        for _ in 0..128 {
            let token = source.session_token().expect("session token");
            let encoded = std::str::from_utf8(token.expose_for_hashing()).expect("ASCII token");
            assert_eq!(encoded.len(), 43);
            assert!(tokens.insert(encoded.to_owned()));
            let VerificationSecret::OneTimeCode(code) = source
                .verification_secret(VerificationChannel::OneTimeCode)
                .expect("one-time code")
            else {
                panic!("one-time code expected");
            };
            assert_eq!(code.expose_for_hashing().len(), 6);
            assert!(code.expose_for_hashing().iter().all(u8::is_ascii_digit));
        }
    }

    #[test]
    fn versioned_aead_keyring_is_strict_and_rotation_bounded() {
        let ring = keyring();
        assert_eq!(ring.active.version, 2);
        assert!(ring.by_version(1).is_some());
        assert!(AeadKeyRing::parse("{}").is_err());
        assert!(AeadKeyRing::parse(&format!(
            r#"{{"active":{{"version":1,"material_hex":"{}"}},"fallback":[],"raw":"forbidden"}}"#,
            "00".repeat(32)
        ))
        .is_err());
        assert!(
            AeadKeyRing::parse(&format!(
                r#"{{"active":{{"version":1,"material_hex":"{}"}},"fallback":[]}}"#,
                "00".repeat(32)
            ))
            .is_err()
        );
        assert!(
            AeadKeyRing::parse(&format!(
                r#"{{"active":{{"version":1,"material_hex":"{}"}},"fallback":[]}}"#,
                "AA".repeat(32)
            ))
            .is_err()
        );
    }

    #[test]
    fn delivery_envelopes_are_authenticated_fixed_size_and_plaintext_free() {
        let sealer = WorkerDeliverySealer {
            key: keyring().active,
        };
        let now = TimestampMillis::new(1_000).expect("time");
        let material = VerificationDeliveryMaterial {
            destination: VerificationDestination::parse("person@example.test")
                .expect("destination"),
            secret: VerificationSecret::OneTimeCode(OneTimeCode::parse("123456").expect("code")),
            purpose: VerificationPurpose::AccountRecovery,
            expires_at: TimestampMillis::new(901_000).expect("expiry"),
        };
        let first = sealer.seal(&material, now).expect("seal");
        let second = sealer.seal(&material, now).expect("seal again");
        assert_eq!(first.sealed_payload().len(), DELIVERY_PAYLOAD_BYTES);
        assert_eq!(second.sealed_payload().len(), DELIVERY_PAYLOAD_BYTES);
        assert_ne!(first.sealed_payload(), second.sealed_payload());
        assert!(
            !first
                .sealed_payload()
                .windows(b"person@example.test".len())
                .any(|window| window == b"person@example.test")
        );
        assert!(
            !first
                .sealed_payload()
                .windows(6)
                .any(|window| window == b"123456")
        );
    }

    #[test]
    fn pending_cookie_roundtrip_rejects_tamper_expiry_and_wrong_key() {
        let cipher = PendingCookieCipher { keys: keyring() };
        let now = TimestampMillis::new(10_000).expect("time");
        let state = PendingAuthState::new(
            PendingJourney::Signup,
            "person@example.test".into(),
            Some("Person".into()),
            now,
        )
        .expect("pending state");
        let debug = format!("{state:?}");
        assert!(!debug.contains("person@example.test"));
        assert!(!debug.contains("Person"));
        assert!(debug.contains("<redacted>"));
        let encoded = cipher.seal(&state).expect("seal pending");
        assert_eq!(cipher.open(&encoded, now).expect("open pending"), state);
        let wrong_key = PendingCookieCipher {
            keys: AeadKeyRing::parse(&format!(
                r#"{{"active":{{"version":2,"material_hex":"{}"}},"fallback":[]}}"#,
                "ef".repeat(32)
            ))
            .expect("alternate key"),
        };
        assert_eq!(
            wrong_key.open(&encoded, now),
            Err(BrowserWebFailure::Invalid)
        );
        let mut tampered = encoded.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(
            cipher
                .open(std::str::from_utf8(&tampered).expect("ASCII"), now)
                .is_err()
        );
        assert_eq!(
            cipher.open(
                &cipher.seal(&state).expect("seal"),
                TimestampMillis::new(state.expires_at_ms).expect("expiry")
            ),
            Err(BrowserWebFailure::Unauthenticated)
        );

        let mut invalid_binding = PendingAuthState::new(
            PendingJourney::SignIn,
            "person@example.test".into(),
            None,
            now,
        )
        .expect("sign-in state");
        invalid_binding.user_id = Some(UserId::new().to_string());
        assert_eq!(
            invalid_binding.validate(now),
            Err(BrowserWebFailure::Invalid)
        );

        let mut overflow = state;
        overflow.issued_at_ms = i64::MIN;
        overflow.expires_at_ms = i64::MAX;
        assert_eq!(overflow.validate(now), Err(BrowserWebFailure::Invalid));
    }

    #[test]
    fn form_decoder_is_exact_single_decode_and_duplicate_safe() {
        let login = parse_form(b"email=Person%40Example.test", &["email"]).expect("form");
        assert_eq!(login["email"], "Person@Example.test");
        assert!(parse_form(b"email=a%40b.test&email=c%40d.test", &["email"]).is_err());
        assert!(parse_form(b"email=a%40b.test&tenant_id=forged", &["email"]).is_err());
        assert!(parse_form(b"email=a%2540b.test", &["email"]).is_ok());
        assert!(normalize_identifier("a%40b.test").is_err());
    }

    #[test]
    fn cookie_contracts_are_host_only_secure_and_redaction_safe() {
        let pending = pending_cookie_header("opaque");
        assert_eq!(
            pending,
            "__Host-frame_auth_pending=opaque; Path=/; Max-Age=900; Secure; HttpOnly; SameSite=Strict"
        );
        assert!(!pending.contains("Domain="));
        assert_eq!(
            clear_cookie_header(SESSION_COOKIE_NAME, true),
            "__Host-frame_session=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict"
        );
    }

    #[test]
    fn scheduled_delivery_batch_has_admission_headroom_inside_the_cookie_ttl() {
        let dispatch_budget = std::hint::black_box(AUTH_DELIVERY_DISPATCH_BUDGET_PER_TICK);
        let statements_per_item = std::hint::black_box(AUTH_DELIVERY_D1_STATEMENTS_PER_ITEM);
        let statement_limit = std::hint::black_box(SCHEDULED_D1_STATEMENT_LIMIT);
        let statement_reserve = std::hint::black_box(SCHEDULED_D1_STATEMENT_RESERVE);
        let admitted = usize::try_from(browser_web_runtime::AUTH_DELIVERY_ADMISSION_PER_MINUTE)
            .unwrap_or(usize::MAX)
            .saturating_mul(BROWSER_AUTH_DELIVERY_ACTION_CLASSES);
        assert!(dispatch_budget > admitted);
        assert_eq!(PENDING_TTL_MS / 60_000, 15);
        assert_eq!(BROWSER_AUTH_DELIVERY_ACTION_CLASSES, 1);
        assert!(dispatch_budget * statements_per_item <= statement_limit - statement_reserve);
    }

    #[test]
    fn base64_url_roundtrip_is_canonical() {
        for length in 1..128 {
            let bytes = (0..length).map(|value| value as u8).collect::<Vec<_>>();
            let encoded = base64_url_encode(&bytes);
            assert_eq!(base64_url_decode(&encoded), Some(bytes));
            assert!(!encoded.contains('='));
        }
        assert!(base64_url_decode("a").is_none());
        assert!(base64_url_decode("abc+").is_none());
    }
}
