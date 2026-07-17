//! Native-owned Instant finalize authority for the Tauri composition root.
//!
//! WebView IPC can name only a native-minted opaque handle and monotonic
//! command sequence. Credentials, Frame origins, tenant/video identities,
//! request hashes, and receipts remain in this service.

use std::fmt;

use frame_client::InstantUiProgressV1;
#[cfg(not(target_arch = "wasm32"))]
use frame_client::{InstantUiErrorCodeV1, InstantUiPhaseV1};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[cfg(not(target_arch = "wasm32"))]
use async_trait::async_trait;
#[cfg(not(target_arch = "wasm32"))]
use frame_authenticated_client::{
    InstantFinalizeReceiptV1, InstantFinalizeRequestV1, InstantFinalizeStateV1,
};
#[cfg(not(target_arch = "wasm32"))]
use ring::rand::{SecureRandom, SystemRandom};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[cfg(not(target_arch = "wasm32"))]
use zeroize::Zeroizing;

#[cfg(not(target_arch = "wasm32"))]
use crate::instant_finalize::{
    DesktopInstantFinalizeClient, DesktopInstantFinalizeError, valid_bearer_token,
};

pub const INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION: u16 = 1;
const HANDLE_BYTES: usize = 32;
#[cfg(not(target_arch = "wasm32"))]
const MAX_HANDLE_MINT_ATTEMPTS: usize = 8;
#[cfg(not(target_arch = "wasm32"))]
const MAX_NATIVE_CONTEXTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstantFinalizeCapabilityState {
    NotConfigured,
    Available,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct InstantFinalizeHandle(String);

impl InstantFinalizeHandle {
    #[cfg(not(target_arch = "wasm32"))]
    fn from_random(bytes: [u8; HANDLE_BYTES]) -> Self {
        let mut value = String::with_capacity(HANDLE_BYTES * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
        }
        Self(value)
    }

    fn parse(value: String) -> Result<Self, InstantFinalizeServiceError> {
        if value.len() != HANDLE_BYTES * 2
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InstantFinalizeServiceError::InvalidEnvelope);
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for InstantFinalizeHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InstantFinalizeHandle(<redacted>)")
    }
}

impl Serialize for InstantFinalizeHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for InstantFinalizeHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("invalid opaque finalize handle"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstantFinalizeCommandActionV1 {
    Finalize,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstantFinalizeCommandV1 {
    pub protocol_version: u16,
    pub action: InstantFinalizeCommandActionV1,
    pub sequence: u64,
    pub handle: InstantFinalizeHandle,
}

impl InstantFinalizeCommandV1 {
    pub fn new(
        handle: InstantFinalizeHandle,
        sequence: u64,
    ) -> Result<Self, InstantFinalizeServiceError> {
        let command = Self {
            protocol_version: INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION,
            action: InstantFinalizeCommandActionV1::Finalize,
            sequence,
            handle,
        };
        command.validate()?;
        Ok(command)
    }

    pub fn validate(&self) -> Result<(), InstantFinalizeServiceError> {
        if self.protocol_version != INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION || self.sequence == 0
        {
            return Err(InstantFinalizeServiceError::InvalidEnvelope);
        }
        Ok(())
    }
}

impl fmt::Debug for InstantFinalizeCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeCommandV1")
            .field("protocol_version", &self.protocol_version)
            .field("action", &self.action)
            .field("sequence", &self.sequence)
            .field("handle", &self.handle)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstantFinalizeServiceResultV1 {
    pub protocol_version: u16,
    pub sequence: u64,
    pub progress: InstantUiProgressV1,
}

impl fmt::Debug for InstantFinalizeServiceResultV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeServiceResultV1")
            .field("protocol_version", &self.protocol_version)
            .field("sequence", &self.sequence)
            .field("progress", &self.progress)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InstantFinalizeRegistrationV1 {
    pub handle: InstantFinalizeHandle,
    pub progress: InstantUiProgressV1,
}

impl fmt::Debug for InstantFinalizeRegistrationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeRegistrationV1")
            .field("handle", &self.handle)
            .field("progress", &self.progress)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InstantFinalizeServiceError {
    #[error("the Instant finalize command envelope is invalid")]
    InvalidEnvelope,
    #[error("Instant finalize is not configured")]
    Unavailable,
    #[error("the Instant finalize handle is unavailable")]
    UnknownHandle,
    #[error("an Instant finalize request is already in flight")]
    Busy,
    #[error("the Instant finalize command sequence was replayed")]
    SequenceReplay,
    #[error("the Instant finalize command sequence contains a gap")]
    SequenceGap,
    #[error("the Instant finalize native authority changed")]
    AuthorityChanged,
    #[error("the Instant finalize authority is sealed")]
    Terminal,
    #[error("the Instant finalize provider response was rejected")]
    ProviderRejected,
    #[error("the operating system random source is unavailable")]
    RandomUnavailable,
    #[error("the native Instant finalize registry is unavailable")]
    RegistryUnavailable,
}

#[cfg(not(target_arch = "wasm32"))]
struct BearerCredential(Zeroizing<String>);

#[cfg(not(target_arch = "wasm32"))]
impl BearerCredential {
    fn new(value: String) -> Result<Self, InstantFinalizeServiceError> {
        let value = Zeroizing::new(value);
        if !valid_bearer_token(value.as_str()) {
            return Err(InstantFinalizeServiceError::ProviderRejected);
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for BearerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BearerCredential(<redacted>)")
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, PartialEq, Eq)]
struct DispatchClaim {
    handle_generation: u64,
    sequence: u64,
    request_sha256: String,
    job_generation: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for DispatchClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DispatchClaim")
            .field("handle_generation", &self.handle_generation)
            .field("sequence", &self.sequence)
            .field("request_sha256", &"<redacted>")
            .field("job_generation", &self.job_generation)
            .finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeFinalizeContext {
    credential: Option<Arc<BearerCredential>>,
    request: InstantFinalizeRequestV1,
    handle_generation: u64,
    last_sequence: u64,
    in_flight: Option<DispatchClaim>,
    sealed: bool,
    progress: InstantUiProgressV1,
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for NativeFinalizeContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeFinalizeContext")
            .field("credential", &self.credential)
            .field("request", &"<redacted>")
            .field("handle_generation", &self.handle_generation)
            .field("last_sequence", &self.last_sequence)
            .field("in_flight", &self.in_flight)
            .field("sealed", &self.sealed)
            .field("progress", &self.progress)
            .finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait NativeInstantFinalizeProvider: Send + Sync {
    async fn dispatch(
        &self,
        bearer_credential: &str,
        request: &InstantFinalizeRequestV1,
    ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError>;
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl NativeInstantFinalizeProvider for DesktopInstantFinalizeClient {
    async fn dispatch(
        &self,
        bearer_credential: &str,
        request: &InstantFinalizeRequestV1,
    ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
        self.dispatch_wire(bearer_credential, request).await
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum ProviderSlot {
    NotConfigured,
    Ready(Arc<dyn NativeInstantFinalizeProvider>),
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for ProviderSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => formatter.write_str("ProviderSlot::NotConfigured"),
            Self::Ready(_) => formatter.write_str("ProviderSlot::Ready(<redacted>)"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct InstantFinalizeServiceInner {
    provider: ProviderSlot,
    contexts: Mutex<HashMap<InstantFinalizeHandle, NativeFinalizeContext>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for InstantFinalizeServiceInner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeServiceInner")
            .field("provider", &self.provider)
            .field("contexts", &"<redacted>")
            .finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct InstantFinalizeService {
    inner: Arc<InstantFinalizeServiceInner>,
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for InstantFinalizeService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeService")
            .field("capability", &self.capability())
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl InstantFinalizeService {
    /// Unconditional release constructor. It does not inspect environment
    /// variables, construct an HTTP client, parse an origin, or own a provider.
    #[must_use]
    pub fn not_configured() -> Self {
        Self {
            inner: Arc::new(InstantFinalizeServiceInner {
                provider: ProviderSlot::NotConfigured,
                contexts: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Native composition hook for a future authenticated session owner and
    /// for deterministic tests. WebView code cannot install a provider.
    #[must_use]
    pub fn with_native_provider(provider: Arc<dyn NativeInstantFinalizeProvider>) -> Self {
        Self {
            inner: Arc::new(InstantFinalizeServiceInner {
                provider: ProviderSlot::Ready(provider),
                contexts: Mutex::new(HashMap::new()),
            }),
        }
    }

    #[must_use]
    pub fn capability(&self) -> InstantFinalizeCapabilityState {
        match &self.inner.provider {
            ProviderSlot::NotConfigured => InstantFinalizeCapabilityState::NotConfigured,
            ProviderSlot::Ready(_) => InstantFinalizeCapabilityState::Available,
        }
    }

    /// Registers native-owned authority and returns only an opaque handle plus
    /// the public-safe progress projection. All sensitive inputs stay in the
    /// registry and are redacted on Debug.
    pub fn register_native_context(
        &self,
        bearer_credential: String,
        request: InstantFinalizeRequestV1,
        progress: InstantUiProgressV1,
    ) -> Result<InstantFinalizeRegistrationV1, InstantFinalizeServiceError> {
        let credential = Arc::new(BearerCredential::new(bearer_credential)?);
        if self.capability() != InstantFinalizeCapabilityState::Available {
            return Err(InstantFinalizeServiceError::Unavailable);
        }
        request
            .validate()
            .map_err(|_| InstantFinalizeServiceError::ProviderRejected)?;
        progress
            .validate()
            .map_err(|_| InstantFinalizeServiceError::ProviderRejected)?;
        if progress.phase != InstantUiPhaseV1::Finalizing {
            return Err(InstantFinalizeServiceError::ProviderRejected);
        }
        let random = SystemRandom::new();
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        if contexts.len() >= MAX_NATIVE_CONTEXTS {
            // Successful/failed terminal contexts are retained briefly so a
            // command cancelled between native completion and runtime commit
            // can reconcile without repeating the network request. Once the
            // strict cap is reached, terminal tombstones are safe to evict:
            // their 256-bit handles are never reused.
            contexts.retain(|_, context| !context.sealed);
        }
        if contexts.len() >= MAX_NATIVE_CONTEXTS {
            return Err(InstantFinalizeServiceError::RegistryUnavailable);
        }
        for _ in 0..MAX_HANDLE_MINT_ATTEMPTS {
            let mut bytes = [0_u8; HANDLE_BYTES];
            random
                .fill(&mut bytes)
                .map_err(|_| InstantFinalizeServiceError::RandomUnavailable)?;
            let handle = InstantFinalizeHandle::from_random(bytes);
            if contexts.contains_key(&handle) {
                continue;
            }
            contexts.insert(
                handle.clone(),
                NativeFinalizeContext {
                    credential: Some(Arc::clone(&credential)),
                    request: request.clone(),
                    handle_generation: 1,
                    last_sequence: 0,
                    in_flight: None,
                    sealed: false,
                    progress,
                },
            );
            return Ok(InstantFinalizeRegistrationV1 { handle, progress });
        }
        Err(InstantFinalizeServiceError::RandomUnavailable)
    }

    /// Native-only cancellation/revocation boundary. It invalidates any
    /// in-flight claim generation, drops the registry's credential reference,
    /// and seals the opaque handle before returning a public-safe status.
    pub fn revoke_native_context(
        &self,
        handle: &InstantFinalizeHandle,
    ) -> Result<InstantUiProgressV1, InstantFinalizeServiceError> {
        let progress = InstantUiProgressV1::new(
            InstantUiPhaseV1::Cancelled,
            None,
            false,
            Some(InstantUiErrorCodeV1::RecordingCancelled),
        )
        .map_err(|_| InstantFinalizeServiceError::ProviderRejected)?;
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        let context = contexts
            .get_mut(handle)
            .ok_or(InstantFinalizeServiceError::UnknownHandle)?;
        context.handle_generation = context
            .handle_generation
            .checked_add(1)
            .ok_or(InstantFinalizeServiceError::AuthorityChanged)?;
        context.in_flight = None;
        context.sealed = true;
        context.credential = None;
        context.progress = progress;
        Ok(progress)
    }

    pub async fn dispatch(
        &self,
        command: InstantFinalizeCommandV1,
    ) -> Result<InstantFinalizeServiceResultV1, InstantFinalizeServiceError> {
        command.validate()?;
        let provider = match &self.inner.provider {
            ProviderSlot::NotConfigured => return Err(InstantFinalizeServiceError::Unavailable),
            ProviderSlot::Ready(provider) => Arc::clone(provider),
        };
        let (material, mut guard) = self.claim(&command)?;
        let response = provider
            .dispatch(material.credential.expose(), &material.request)
            .await;
        let result = match response {
            Ok(receipt) => self.finish_receipt(&material, receipt),
            Err(error) => self.finish_provider_error(&material, error),
        };
        if result.is_ok() || !self.claim_is_current(&material) {
            guard.disarm();
        }
        result
    }

    /// Returns a native result already committed for exactly this command.
    /// The Tauri composition root uses this only after its runtime preflight,
    /// repairing cancellation in the narrow post-network/pre-UI-commit gap
    /// without issuing a second request or accepting a WebView replay.
    pub fn reconciled_result(
        &self,
        command: &InstantFinalizeCommandV1,
    ) -> Result<Option<InstantFinalizeServiceResultV1>, InstantFinalizeServiceError> {
        command.validate()?;
        if self.capability() != InstantFinalizeCapabilityState::Available {
            return Err(InstantFinalizeServiceError::Unavailable);
        }
        let contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        let context = contexts
            .get(&command.handle)
            .ok_or(InstantFinalizeServiceError::UnknownHandle)?;
        if context.in_flight.is_some() {
            return Err(InstantFinalizeServiceError::Busy);
        }
        if command.sequence == context.last_sequence {
            return Ok(Some(InstantFinalizeServiceResultV1 {
                protocol_version: INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION,
                sequence: context.last_sequence,
                progress: context.progress,
            }));
        }
        Ok(None)
    }

    /// Removes a terminal native context after the runtime has committed the
    /// corresponding public-safe status. Active contexts are never removed by
    /// this method.
    pub fn forget_terminal_context(
        &self,
        handle: &InstantFinalizeHandle,
    ) -> Result<(), InstantFinalizeServiceError> {
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        if contexts.get(handle).is_some_and(|context| context.sealed) {
            contexts.remove(handle);
        }
        Ok(())
    }

    fn claim(
        &self,
        command: &InstantFinalizeCommandV1,
    ) -> Result<(DispatchMaterial, DispatchClaimGuard), InstantFinalizeServiceError> {
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        let context = contexts
            .get_mut(&command.handle)
            .ok_or(InstantFinalizeServiceError::UnknownHandle)?;
        if context.in_flight.is_some() {
            return Err(InstantFinalizeServiceError::Busy);
        }
        if context.sealed {
            return Err(InstantFinalizeServiceError::Terminal);
        }
        let expected = context
            .last_sequence
            .checked_add(1)
            .ok_or(InstantFinalizeServiceError::SequenceGap)?;
        if command.sequence < expected {
            return Err(InstantFinalizeServiceError::SequenceReplay);
        }
        if command.sequence > expected {
            return Err(InstantFinalizeServiceError::SequenceGap);
        }
        context
            .request
            .validate()
            .map_err(|_| InstantFinalizeServiceError::AuthorityChanged)?;
        let claim = DispatchClaim {
            handle_generation: context.handle_generation,
            sequence: command.sequence,
            request_sha256: context.request.request_sha256.clone(),
            job_generation: context.request.job_generation,
        };
        let credential = Arc::clone(
            context
                .credential
                .as_ref()
                .ok_or(InstantFinalizeServiceError::Terminal)?,
        );
        context.in_flight = Some(claim.clone());
        let material = DispatchMaterial {
            handle: command.handle.clone(),
            credential,
            request: context.request.clone(),
            claim: claim.clone(),
        };
        let guard = DispatchClaimGuard {
            inner: Arc::clone(&self.inner),
            handle: command.handle.clone(),
            claim,
            armed: true,
        };
        Ok((material, guard))
    }

    fn finish_receipt(
        &self,
        material: &DispatchMaterial,
        receipt: InstantFinalizeReceiptV1,
    ) -> Result<InstantFinalizeServiceResultV1, InstantFinalizeServiceError> {
        if receipt.validate_for(&material.request).is_err() {
            self.seal_exact(material, recovery_required_progress()?)?;
            return Err(InstantFinalizeServiceError::ProviderRejected);
        }
        let progress = match receipt.state {
            InstantFinalizeStateV1::Pending => {
                InstantUiProgressV1::new(InstantUiPhaseV1::Finalizing, None, false, None)
            }
            InstantFinalizeStateV1::Published => {
                InstantUiProgressV1::new(InstantUiPhaseV1::ShareReady, Some(10_000), false, None)
            }
        }
        .map_err(|_| InstantFinalizeServiceError::ProviderRejected)?;
        self.apply_exact(
            material,
            progress,
            receipt.state == InstantFinalizeStateV1::Published,
        )
    }

    fn finish_provider_error(
        &self,
        material: &DispatchMaterial,
        error: DesktopInstantFinalizeError,
    ) -> Result<InstantFinalizeServiceResultV1, InstantFinalizeServiceError> {
        if !retryable_provider_error(error) {
            self.seal_exact(material, recovery_required_progress()?)?;
            return Err(InstantFinalizeServiceError::ProviderRejected);
        }
        let progress = InstantUiProgressV1::new(
            InstantUiPhaseV1::Finalizing,
            None,
            true,
            Some(InstantUiErrorCodeV1::FinalizeDelayed),
        )
        .map_err(|_| InstantFinalizeServiceError::ProviderRejected)?;
        self.apply_exact(material, progress, false)
    }

    fn apply_exact(
        &self,
        material: &DispatchMaterial,
        progress: InstantUiProgressV1,
        seal: bool,
    ) -> Result<InstantFinalizeServiceResultV1, InstantFinalizeServiceError> {
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        let context = contexts
            .get_mut(&material.handle)
            .ok_or(InstantFinalizeServiceError::AuthorityChanged)?;
        require_exact_authority(context, material)?;
        context
            .request
            .validate()
            .map_err(|_| InstantFinalizeServiceError::AuthorityChanged)?;
        context.last_sequence = material.claim.sequence;
        context.in_flight = None;
        context.sealed = seal;
        if seal {
            context.credential = None;
        }
        context.progress = progress;
        Ok(InstantFinalizeServiceResultV1 {
            protocol_version: INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION,
            sequence: material.claim.sequence,
            progress,
        })
    }

    fn seal_exact(
        &self,
        material: &DispatchMaterial,
        progress: InstantUiProgressV1,
    ) -> Result<(), InstantFinalizeServiceError> {
        let mut contexts = self
            .inner
            .contexts
            .lock()
            .map_err(|_| InstantFinalizeServiceError::RegistryUnavailable)?;
        let context = contexts
            .get_mut(&material.handle)
            .ok_or(InstantFinalizeServiceError::AuthorityChanged)?;
        require_exact_authority(context, material)?;
        context.last_sequence = material.claim.sequence;
        context.in_flight = None;
        context.sealed = true;
        context.credential = None;
        context.progress = progress;
        Ok(())
    }

    fn claim_is_current(&self, material: &DispatchMaterial) -> bool {
        self.inner.contexts.lock().ok().is_some_and(|contexts| {
            contexts
                .get(&material.handle)
                .is_some_and(|context| exact_authority(context, material))
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_required_progress() -> Result<InstantUiProgressV1, InstantFinalizeServiceError> {
    InstantUiProgressV1::new(
        InstantUiPhaseV1::RecoveryRequired,
        None,
        false,
        Some(InstantUiErrorCodeV1::RecordingRecoveryRequired),
    )
    .map_err(|_| InstantFinalizeServiceError::ProviderRejected)
}

#[cfg(not(target_arch = "wasm32"))]
fn retryable_provider_error(error: DesktopInstantFinalizeError) -> bool {
    match error {
        DesktopInstantFinalizeError::TransportUnavailable
        | DesktopInstantFinalizeError::DeadlineExceeded => true,
        DesktopInstantFinalizeError::ApiRejected(status) => {
            matches!(status, 408 | 425 | 429 | 500..=599)
        }
        DesktopInstantFinalizeError::InvalidContract
        | DesktopInstantFinalizeError::InvalidCredential
        | DesktopInstantFinalizeError::RedirectRejected
        | DesktopInstantFinalizeError::ResponseTooLarge
        | DesktopInstantFinalizeError::UnsupportedContentType
        | DesktopInstantFinalizeError::MalformedResponse
        | DesktopInstantFinalizeError::InvalidResponse => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct DispatchMaterial {
    handle: InstantFinalizeHandle,
    credential: Arc<BearerCredential>,
    request: InstantFinalizeRequestV1,
    claim: DispatchClaim,
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for DispatchMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DispatchMaterial")
            .field("handle", &self.handle)
            .field("credential", &self.credential)
            .field("request", &"<redacted>")
            .field("claim", &self.claim)
            .finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct DispatchClaimGuard {
    inner: Arc<InstantFinalizeServiceInner>,
    handle: InstantFinalizeHandle,
    claim: DispatchClaim,
    armed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl DispatchClaimGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for DispatchClaimGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(mut contexts) = self.inner.contexts.lock() else {
            return;
        };
        let Some(context) = contexts.get_mut(&self.handle) else {
            return;
        };
        if context.in_flight.as_ref() == Some(&self.claim)
            && context.handle_generation == self.claim.handle_generation
            && context.request.request_sha256 == self.claim.request_sha256
            && context.request.job_generation == self.claim.job_generation
        {
            context.in_flight = None;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn require_exact_authority(
    context: &NativeFinalizeContext,
    material: &DispatchMaterial,
) -> Result<(), InstantFinalizeServiceError> {
    if exact_authority(context, material) {
        Ok(())
    } else {
        Err(InstantFinalizeServiceError::AuthorityChanged)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exact_authority(context: &NativeFinalizeContext, material: &DispatchMaterial) -> bool {
    context.in_flight.as_ref() == Some(&material.claim)
        && context.handle_generation == material.claim.handle_generation
        && context.request.request_sha256 == material.claim.request_sha256
        && context.request.job_generation == material.claim.job_generation
        && material.request.request_sha256 == material.claim.request_sha256
        && material.request.job_generation == material.claim.job_generation
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::{
        future::pending,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::sync::Notify;

    use frame_authenticated_client::INSTANT_FINALIZE_SCHEMA_VERSION;

    use super::*;

    const BEARER: &str = "native-secret-bearer-credential-000000000000000000";

    fn request() -> InstantFinalizeRequestV1 {
        request_with_generation(1)
    }

    fn request_with_generation(job_generation: u64) -> InstantFinalizeRequestV1 {
        InstantFinalizeRequestV1::new(
            "018f47a6-7b1c-7f55-8f39-8f8a86900001".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900002".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900003".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900004".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900005".into(),
            "a".repeat(64),
            "b".repeat(64),
            "018f47a6-7b1c-7f55-8f39-8f8a86900007".into(),
            job_generation,
        )
        .expect("request")
    }

    fn initial_progress() -> InstantUiProgressV1 {
        InstantUiProgressV1::new(InstantUiPhaseV1::Finalizing, None, false, None).expect("progress")
    }

    fn published(request: &InstantFinalizeRequestV1) -> InstantFinalizeReceiptV1 {
        InstantFinalizeReceiptV1 {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            state: InstantFinalizeStateV1::Published,
            request_sha256: request.request_sha256.clone(),
            publication_id: Some("018f47a6-7b1c-7f55-8f39-8f8a86900008".into()),
            job_id: request.job_id.clone(),
            job_generation: request.job_generation,
            upload_id: request.upload_id.clone(),
            object_version: request.object_version.clone(),
            distribution_eligible: true,
        }
    }

    fn pending_receipt(request: &InstantFinalizeRequestV1) -> InstantFinalizeReceiptV1 {
        InstantFinalizeReceiptV1 {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            state: InstantFinalizeStateV1::Pending,
            request_sha256: request.request_sha256.clone(),
            publication_id: None,
            job_id: request.job_id.clone(),
            job_generation: request.job_generation,
            upload_id: request.upload_id.clone(),
            object_version: request.object_version.clone(),
            distribution_eligible: false,
        }
    }

    #[derive(Debug)]
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct ErrorProvider {
        calls: Arc<AtomicUsize>,
        error: DesktopInstantFinalizeError,
    }

    #[derive(Debug)]
    struct PendingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for PendingProvider {
        async fn dispatch(
            &self,
            _bearer_credential: &str,
            request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(pending_receipt(request))
        }
    }

    #[derive(Debug)]
    struct MismatchedReceiptProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for MismatchedReceiptProvider {
        async fn dispatch(
            &self,
            _bearer_credential: &str,
            request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut receipt = published(request);
            receipt.request_sha256 = "f".repeat(64);
            Ok(receipt)
        }
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for ErrorProvider {
        async fn dispatch(
            &self,
            _bearer_credential: &str,
            _request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(self.error)
        }
    }

    #[derive(Debug)]
    struct GatedProvider {
        calls: Arc<AtomicUsize>,
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for GatedProvider {
        async fn dispatch(
            &self,
            _bearer_credential: &str,
            request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.notified().await;
            Ok(published(request))
        }
    }

    #[derive(Debug)]
    struct CancelThenPublishProvider {
        calls: Arc<AtomicUsize>,
        started: Arc<Notify>,
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for CancelThenPublishProvider {
        async fn dispatch(
            &self,
            _bearer_credential: &str,
            request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                self.started.notify_one();
                return pending().await;
            }
            Ok(published(request))
        }
    }

    #[async_trait]
    impl NativeInstantFinalizeProvider for CountingProvider {
        async fn dispatch(
            &self,
            bearer_credential: &str,
            request: &InstantFinalizeRequestV1,
        ) -> Result<InstantFinalizeReceiptV1, DesktopInstantFinalizeError> {
            assert_eq!(bearer_credential, BEARER);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(published(request))
        }
    }

    #[test]
    fn strict_envelope_rejects_unknown_fields_versions_zero_sequence_and_bad_handles() {
        let unknown = format!(
            r#"{{"protocol_version":1,"action":"finalize","sequence":1,"handle":"{}","tenant_id":"forbidden"}}"#,
            "a".repeat(64)
        );
        assert!(serde_json::from_str::<InstantFinalizeCommandV1>(&unknown).is_err());
        for invalid in [
            format!(
                r#"{{"protocol_version":2,"action":"finalize","sequence":1,"handle":"{}"}}"#,
                "a".repeat(64)
            ),
            format!(
                r#"{{"protocol_version":1,"action":"finalize","sequence":0,"handle":"{}"}}"#,
                "a".repeat(64)
            ),
            r#"{"protocol_version":1,"action":"finalize","sequence":1,"handle":"session-id"}"#
                .into(),
        ] {
            if let Ok(value) = serde_json::from_str::<InstantFinalizeCommandV1>(&invalid) {
                assert!(value.validate().is_err());
            }
        }
    }

    #[tokio::test]
    async fn not_configured_fails_before_handle_lookup_or_provider_dispatch() {
        let service = InstantFinalizeService::not_configured();
        assert_eq!(
            service.capability(),
            InstantFinalizeCapabilityState::NotConfigured
        );
        let handle = InstantFinalizeHandle::parse("a".repeat(64)).expect("opaque handle");
        let result = service
            .dispatch(InstantFinalizeCommandV1::new(handle, 1).expect("command"))
            .await;
        assert_eq!(result, Err(InstantFinalizeServiceError::Unavailable));
        assert!(matches!(
            service.inner.provider,
            ProviderSlot::NotConfigured
        ));
        assert!(service.inner.contexts.lock().expect("registry").is_empty());
    }

    #[tokio::test]
    async fn native_registry_resolves_secret_and_returns_only_public_progress() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = InstantFinalizeService::with_native_provider(Arc::new(CountingProvider {
            calls: Arc::clone(&calls),
        }));
        let native_request = request();
        let registration = service
            .register_native_context(BEARER.into(), native_request.clone(), initial_progress())
            .expect("native registration");
        let command =
            InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command");
        let command_json = serde_json::to_string(&command).expect("command JSON");
        for forbidden in [
            BEARER,
            &native_request.tenant_id,
            &native_request.session_id,
            &native_request.request_sha256,
        ] {
            assert!(!command_json.contains(forbidden));
            assert!(!format!("{command:?}").contains(forbidden));
        }
        let result = service.dispatch(command).await.expect("dispatch");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.progress.phase, InstantUiPhaseV1::ShareReady);
        let result_json = serde_json::to_string(&result).expect("result JSON");
        for forbidden in [
            BEARER,
            &native_request.tenant_id,
            &native_request.session_id,
            &native_request.request_sha256,
            "018f47a6-7b1c-7f55-8f39-8f8a86900008",
        ] {
            assert!(!result_json.contains(forbidden));
            assert!(!format!("{result:?}").contains(forbidden));
        }
        {
            let contexts = service.inner.contexts.lock().expect("registry");
            let context = contexts
                .get(&registration.handle)
                .expect("terminal tombstone");
            assert!(context.sealed);
            assert!(context.credential.is_none());
        }
        assert_eq!(
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle, 2)
                        .expect("terminal command"),
                )
                .await,
            Err(InstantFinalizeServiceError::Terminal)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replay_and_sequence_gap_fail_without_provider_calls() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = InstantFinalizeService::with_native_provider(Arc::new(PendingProvider {
            calls: Arc::clone(&calls),
        }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        service
            .dispatch(InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("first"))
            .await
            .expect("first dispatch");
        assert_eq!(
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("replay"),
                )
                .await,
            Err(InstantFinalizeServiceError::SequenceReplay)
        );
        assert_eq!(
            service
                .dispatch(InstantFinalizeCommandV1::new(registration.handle, 3).expect("gap"))
                .await,
            Err(InstantFinalizeServiceError::SequenceGap)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn handle_is_256_bit_lower_hex_and_all_debug_surfaces_are_redacted() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service =
            InstantFinalizeService::with_native_provider(Arc::new(CountingProvider { calls }));
        let native_request = request();
        let registration = service
            .register_native_context(BEARER.into(), native_request.clone(), initial_progress())
            .expect("registration");
        let encoded = serde_json::to_value(&registration.handle).expect("handle JSON");
        let value = encoded.as_str().expect("handle string");
        assert_eq!(value.len(), 64);
        assert!(
            value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        let debug = format!("{service:?} {registration:?}");
        assert!(!debug.contains(value));
        assert!(!debug.contains(BEARER));
        assert!(!debug.contains(&native_request.request_sha256));
    }

    #[tokio::test]
    async fn permanent_provider_errors_seal_without_another_network_attempt() {
        for error in [
            DesktopInstantFinalizeError::MalformedResponse,
            DesktopInstantFinalizeError::ApiRejected(401),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let service = InstantFinalizeService::with_native_provider(Arc::new(ErrorProvider {
                calls: Arc::clone(&calls),
                error,
            }));
            let registration = service
                .register_native_context(BEARER.into(), request(), initial_progress())
                .expect("registration");
            assert_eq!(
                service
                    .dispatch(
                        InstantFinalizeCommandV1::new(registration.handle.clone(), 1)
                            .expect("first"),
                    )
                    .await,
                Err(InstantFinalizeServiceError::ProviderRejected)
            );
            assert_eq!(
                service
                    .dispatch(
                        InstantFinalizeCommandV1::new(registration.handle, 2)
                            .expect("terminal retry"),
                    )
                    .await,
                Err(InstantFinalizeServiceError::Terminal)
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn transient_statuses_return_retryable_progress_and_consume_sequence() {
        for status in [429, 500] {
            let calls = Arc::new(AtomicUsize::new(0));
            let service = InstantFinalizeService::with_native_provider(Arc::new(ErrorProvider {
                calls: Arc::clone(&calls),
                error: DesktopInstantFinalizeError::ApiRejected(status),
            }));
            let registration = service
                .register_native_context(BEARER.into(), request(), initial_progress())
                .expect("registration");
            for sequence in 1..=2 {
                let result = service
                    .dispatch(
                        InstantFinalizeCommandV1::new(registration.handle.clone(), sequence)
                            .expect("retry command"),
                    )
                    .await
                    .expect("retryable progress");
                assert!(result.progress.retrying);
                assert_eq!(
                    result.progress.error,
                    Some(InstantUiErrorCodeV1::FinalizeDelayed)
                );
            }
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn invalid_registration_is_rejected_before_registry_insertion() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = InstantFinalizeService::with_native_provider(Arc::new(CountingProvider {
            calls: Arc::clone(&calls),
        }));
        let mut invalid = request();
        invalid.request_sha256 = "f".repeat(64);
        assert_eq!(
            service.register_native_context(BEARER.into(), invalid, initial_progress()),
            Err(InstantFinalizeServiceError::ProviderRejected)
        );
        for terminal_progress in [
            InstantUiProgressV1::new(InstantUiPhaseV1::ShareReady, Some(10_000), false, None)
                .expect("share ready"),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::Cancelled,
                None,
                false,
                Some(InstantUiErrorCodeV1::RecordingCancelled),
            )
            .expect("cancelled"),
        ] {
            assert_eq!(
                service.register_native_context(BEARER.into(), request(), terminal_progress),
                Err(InstantFinalizeServiceError::ProviderRejected)
            );
        }
        assert!(service.inner.contexts.lock().expect("registry").is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn committed_result_reconciles_post_network_cancellation_gap_without_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = InstantFinalizeService::with_native_provider(Arc::new(PendingProvider {
            calls: Arc::clone(&calls),
        }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        let command = InstantFinalizeCommandV1::new(registration.handle, 1).expect("command");
        let first = service
            .dispatch(command.clone())
            .await
            .expect("first dispatch");
        let reconciled = service
            .reconciled_result(&command)
            .expect("reconciliation")
            .expect("committed result");
        assert_eq!(reconciled, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn terminal_context_registry_remains_strictly_bounded() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = InstantFinalizeService::with_native_provider(Arc::new(CountingProvider {
            calls: Arc::clone(&calls),
        }));
        for _ in 0..(MAX_NATIVE_CONTEXTS * 2 + 1) {
            let registration = service
                .register_native_context(BEARER.into(), request(), initial_progress())
                .expect("bounded registration");
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle, 1).expect("publish command"),
                )
                .await
                .expect("published");
            assert!(service.inner.contexts.lock().expect("registry").len() <= MAX_NATIVE_CONTEXTS);
        }
        assert_eq!(calls.load(Ordering::SeqCst), MAX_NATIVE_CONTEXTS * 2 + 1);
    }

    #[tokio::test]
    async fn mismatched_receipt_seals_context_and_drops_credential() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service =
            InstantFinalizeService::with_native_provider(Arc::new(MismatchedReceiptProvider {
                calls: Arc::clone(&calls),
            }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        assert_eq!(
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command"),
                )
                .await,
            Err(InstantFinalizeServiceError::ProviderRejected)
        );
        {
            let contexts = service.inner.contexts.lock().expect("registry");
            let context = contexts.get(&registration.handle).expect("tombstone");
            assert!(context.sealed);
            assert!(context.credential.is_none());
        }
        assert_eq!(
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle, 2)
                        .expect("terminal command"),
                )
                .await,
            Err(InstantFinalizeServiceError::Terminal)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_duplicate_is_busy_and_no_registry_guard_crosses_await() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let service = InstantFinalizeService::with_native_provider(Arc::new(GatedProvider {
            calls: Arc::clone(&calls),
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        let command =
            InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command");
        let running_service = service.clone();
        let running_command = command.clone();
        let task = tokio::spawn(async move { running_service.dispatch(running_command).await });
        started.notified().await;
        assert_eq!(
            service.dispatch(command).await,
            Err(InstantFinalizeServiceError::Busy)
        );
        release.notify_one();
        task.await.expect("task").expect("published");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_dispatch_releases_exact_claim_for_same_sequence_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let service =
            InstantFinalizeService::with_native_provider(Arc::new(CancelThenPublishProvider {
                calls: Arc::clone(&calls),
                started: Arc::clone(&started),
            }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        let command =
            InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command");
        let running_service = service.clone();
        let running_command = command.clone();
        let task = tokio::spawn(async move { running_service.dispatch(running_command).await });
        started.notified().await;
        task.abort();
        assert!(task.await.expect_err("cancelled").is_cancelled());
        tokio::task::yield_now().await;
        let result = service
            .dispatch(command)
            .await
            .expect("retry after cancellation");
        assert_eq!(result.progress.phase, InstantUiPhaseV1::ShareReady);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stale_completion_cannot_clear_or_overwrite_newer_native_claim() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let service = InstantFinalizeService::with_native_provider(Arc::new(GatedProvider {
            calls,
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        let command =
            InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command");
        let running_service = service.clone();
        let task = tokio::spawn(async move { running_service.dispatch(command).await });
        started.notified().await;

        let newer = request_with_generation(2);
        let newer_claim = DispatchClaim {
            handle_generation: 2,
            sequence: 1,
            request_sha256: newer.request_sha256.clone(),
            job_generation: newer.job_generation,
        };
        {
            let mut contexts = service.inner.contexts.lock().expect("registry");
            let context = contexts
                .get_mut(&registration.handle)
                .expect("native context");
            context.handle_generation = 2;
            context.request = newer;
            context.in_flight = Some(newer_claim.clone());
        }
        release.notify_one();
        assert_eq!(
            task.await.expect("task"),
            Err(InstantFinalizeServiceError::AuthorityChanged)
        );
        let contexts = service.inner.contexts.lock().expect("registry");
        assert_eq!(
            contexts
                .get(&registration.handle)
                .and_then(|context| context.in_flight.as_ref()),
            Some(&newer_claim)
        );
    }

    #[tokio::test]
    async fn native_revoke_seals_handle_and_stale_inflight_cannot_resurrect() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let service = InstantFinalizeService::with_native_provider(Arc::new(GatedProvider {
            calls: Arc::clone(&calls),
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }));
        let registration = service
            .register_native_context(BEARER.into(), request(), initial_progress())
            .expect("registration");
        let running_service = service.clone();
        let command =
            InstantFinalizeCommandV1::new(registration.handle.clone(), 1).expect("command");
        let task = tokio::spawn(async move { running_service.dispatch(command).await });
        started.notified().await;

        let cancelled = service
            .revoke_native_context(&registration.handle)
            .expect("native revoke");
        assert_eq!(cancelled.phase, InstantUiPhaseV1::Cancelled);
        release.notify_one();
        assert_eq!(
            task.await.expect("task"),
            Err(InstantFinalizeServiceError::AuthorityChanged)
        );
        assert_eq!(
            service
                .dispatch(
                    InstantFinalizeCommandV1::new(registration.handle.clone(), 1)
                        .expect("revoked command"),
                )
                .await,
            Err(InstantFinalizeServiceError::Terminal)
        );
        let contexts = service.inner.contexts.lock().expect("registry");
        let context = contexts.get(&registration.handle).expect("tombstone");
        assert!(context.sealed);
        assert!(context.credential.is_none());
        assert!(context.in_flight.is_none());
        assert_eq!(context.progress.phase, InstantUiPhaseV1::Cancelled);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
