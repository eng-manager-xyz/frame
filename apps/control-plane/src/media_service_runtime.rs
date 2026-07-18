//! Durable Worker orchestration for the bounded Cloudflare Media lane.
//!
//! Provider interop stays in `cloudflare_media`; this module owns trusted-probe
//! routing, D1 attempt fencing, cancellation, fallback, and reconciliation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Env, Result};

use crate::cloudflare_media::{
    CloudflareCancellationPlan, CloudflareMediaAdapterError, CloudflareMediaBindingAdapter,
    CloudflareMediaFormat, CloudflareMediaMode, CloudflareMediaRequest, CloudflareStagedOutput,
};
use crate::commands::hex;
use crate::contracts::{
    ManagedMediaFormat, ManagedMediaMode, MediaJobRequest, MediaResizeFit, MediaTransformRequest,
};

const LEASE_MS: i64 = 120_000;
const CANCELLATION_SETTLE_MS: i64 = LEASE_MS * 2;
const MAX_ATTEMPTS: i64 = 3;
const MAX_INPUT_BYTES_EXCLUSIVE: u64 = 100_000_000;
const MAX_INPUT_DURATION_MS: u64 = 600_000;
const MAX_INPUT_WIDTH: u32 = 7_680;
const MAX_INPUT_HEIGHT: u32 = 4_320;
const MIN_OUTPUT_DURATION_MS: u64 = 1_000;
const MAX_OUTPUT_DURATION_MS: u64 = 60_000;
const MIN_OUTPUT_DIMENSION: u32 = 10;
const MAX_OUTPUT_DIMENSION: u32 = 2_000;
const MAX_IMAGE_COUNT: u16 = 100;
const MAX_OUTPUT_BYTES: u64 = 32_000_000;
const MAX_NATIVE_PROBE_BYTES: usize = 64 * 1_024;
const MAX_NATIVE_PROBE_DURATION_MS: u64 = 14_400_000;
const MAX_NATIVE_PROBE_WIDTH: u32 = 7_680;
const MAX_NATIVE_PROBE_HEIGHT: u32 = 4_320;
const MAX_NATIVE_PROBE_FRAMES: u64 = 1_300_000;
const MAX_NATIVE_PROBE_TRACKS: u16 = 32;
const MAX_NATIVE_PROBE_DECODED_BYTES: u64 = 64_000_000_000;
const MAX_PROBE_FRAME_RATE: u64 = 240;
const MAX_AUDIO_SAMPLE_RATE: u64 = 192_000;
const MAX_AUDIO_CHANNELS: u64 = 8;
const MAX_AUDIO_SAMPLE_BYTES: u64 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProbeManifestV1 {
    schema_version: u16,
    profile: String,
    container: String,
    video_codec: String,
    audio_codec: String,
    duration_ms: u64,
    width: u32,
    height: u32,
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
    track_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedNativeProbeV1 {
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub decoded_bytes_upper_bound: u64,
    pub frame_count_upper_bound: u64,
    pub track_count: u16,
}

pub(crate) fn verify_native_probe_v1(
    bytes: &[u8],
    source_content_type: &str,
) -> std::result::Result<VerifiedNativeProbeV1, ManagedRuntimeError> {
    if bytes.is_empty() || bytes.len() > MAX_NATIVE_PROBE_BYTES {
        return Err(ManagedRuntimeError::InvalidRequest);
    }
    let manifest: ProbeManifestV1 =
        serde_json::from_slice(bytes).map_err(|_| ManagedRuntimeError::InvalidRequest)?;
    // Require one byte representation so the immutable output checksum is the
    // probe identity rather than merely one of many equivalent JSON encodings.
    let canonical =
        serde_json::to_vec(&manifest).map_err(|_| ManagedRuntimeError::InvalidRequest)?;
    if canonical != bytes {
        return Err(ManagedRuntimeError::InvalidRequest);
    }
    let expected_container = match source_content_type {
        "video/mp4" => "mp4",
        "video/quicktime" => "quicktime",
        "video/webm" => "webm",
        "video/x-matroska" => "matroska",
        _ => return Err(ManagedRuntimeError::InvalidRequest),
    };
    let video_codec_valid = matches!(
        manifest.video_codec.as_str(),
        "h264" | "h265" | "vp8" | "vp9" | "av1" | "prores" | "theora" | "unknown"
    );
    let audio_codec_valid = matches!(
        manifest.audio_codec.as_str(),
        "aac" | "mp3" | "opus" | "vorbis" | "flac" | "pcm" | "none" | "unknown"
    );
    let minimum_tracks = if manifest.audio_codec == "none" { 1 } else { 2 };
    let fps_numerator = u64::from(manifest.frame_rate_numerator);
    let fps_denominator = u64::from(manifest.frame_rate_denominator);
    if manifest.schema_version != 1
        || manifest.profile != "probe_v1"
        || manifest.container != expected_container
        || !video_codec_valid
        || !audio_codec_valid
        || manifest.duration_ms == 0
        || manifest.duration_ms > MAX_NATIVE_PROBE_DURATION_MS
        || !(1..=MAX_NATIVE_PROBE_WIDTH).contains(&manifest.width)
        || !(1..=MAX_NATIVE_PROBE_HEIGHT).contains(&manifest.height)
        || fps_numerator == 0
        || fps_denominator == 0
        || fps_numerator > fps_denominator.saturating_mul(MAX_PROBE_FRAME_RATE)
        || manifest.track_count < minimum_tracks
        || manifest.track_count > MAX_NATIVE_PROBE_TRACKS
    {
        return Err(ManagedRuntimeError::InvalidRequest);
    }

    let frame_denominator = 1_000_u128
        .checked_mul(u128::from(fps_denominator))
        .ok_or(ManagedRuntimeError::InvalidRequest)?;
    let frame_numerator = u128::from(manifest.duration_ms)
        .checked_mul(u128::from(fps_numerator))
        .ok_or(ManagedRuntimeError::InvalidRequest)?;
    let frames = frame_numerator
        .checked_add(frame_denominator - 1)
        .and_then(|value| value.checked_div(frame_denominator))
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| (1..=MAX_NATIVE_PROBE_FRAMES).contains(value))
        .ok_or(ManagedRuntimeError::InvalidRequest)?;
    let video_bytes = u128::from(frames)
        .checked_mul(u128::from(manifest.width))
        .and_then(|value| value.checked_mul(u128::from(manifest.height)))
        .and_then(|value| value.checked_mul(4))
        .ok_or(ManagedRuntimeError::InvalidRequest)?;
    let audio_bytes = if manifest.audio_codec == "none" {
        0
    } else {
        u128::from(manifest.duration_ms)
            .checked_mul(u128::from(MAX_AUDIO_SAMPLE_RATE))
            .and_then(|value| value.checked_mul(u128::from(MAX_AUDIO_CHANNELS)))
            .and_then(|value| value.checked_mul(u128::from(MAX_AUDIO_SAMPLE_BYTES)))
            .and_then(|value| value.checked_add(999))
            .and_then(|value| value.checked_div(1_000))
            .ok_or(ManagedRuntimeError::InvalidRequest)?
    };
    let decoded_bytes = video_bytes
        .checked_add(audio_bytes)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| (1..=MAX_NATIVE_PROBE_DECODED_BYTES).contains(value))
        .ok_or(ManagedRuntimeError::InvalidRequest)?;

    Ok(VerifiedNativeProbeV1 {
        container: manifest.container,
        video_codec: manifest.video_codec,
        audio_codec: manifest.audio_codec,
        duration_ms: manifest.duration_ms,
        width: manifest.width,
        height: manifest.height,
        frame_rate_numerator: manifest.frame_rate_numerator,
        frame_rate_denominator: manifest.frame_rate_denominator,
        decoded_bytes_upper_bound: decoded_bytes,
        frame_count_upper_bound: frames,
        track_count: manifest.track_count,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MediaSourceProbeRow {
    pub organization_id: String,
    pub video_id: String,
    pub source_version: i64,
    pub source_object_key: String,
    pub source_checksum_sha256: String,
    pub source_bytes: i64,
    pub source_content_type: String,
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub duration_ms: i64,
    pub width: i64,
    pub height: i64,
    pub decoded_bytes_upper_bound: i64,
    pub frame_count_upper_bound: i64,
    pub track_count: i64,
    pub probe_digest: String,
    pub trust: String,
    pub state: String,
}

impl MediaSourceProbeRow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_exact_source(
        &self,
        tenant_id: &str,
        video_id: &str,
        source_version: u32,
        source_key: &str,
        source_bytes: i64,
        source_checksum: &str,
        source_content_type: &str,
    ) -> std::result::Result<(), ManagedRuntimeError> {
        if self.organization_id != tenant_id
            || self.video_id != video_id
            || self.source_version != i64::from(source_version)
            || self.source_object_key != source_key
            || self.source_bytes != source_bytes
            || self.source_checksum_sha256 != source_checksum
            || self.source_content_type != source_content_type
            || self.trust != "verified_native_probe"
            || self.state != "verified"
            || !valid_sha256(&self.source_checksum_sha256)
            || !valid_sha256(&self.probe_digest)
            || self.source_bytes <= 0
            || self.duration_ms <= 0
            || self.width <= 0
            || self.height <= 0
            || self.decoded_bytes_upper_bound <= 0
            || self.frame_count_upper_bound <= 0
            || self.track_count <= 0
        {
            return Err(ManagedRuntimeError::UntrustedProbe);
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn supports_managed(&self, transform: &MediaTransformRequest) -> bool {
        let (Ok(bytes), Ok(duration), Ok(width), Ok(height)) = (
            u64::try_from(self.source_bytes),
            u64::try_from(self.duration_ms),
            u32::try_from(self.width),
            u32::try_from(self.height),
        ) else {
            return false;
        };
        self.source_content_type == "video/mp4"
            && self.container == "mp4"
            && self.video_codec == "h264"
            && matches!(self.audio_codec.as_str(), "aac" | "mp3" | "none")
            && bytes < MAX_INPUT_BYTES_EXCLUSIVE
            && duration <= MAX_INPUT_DURATION_MS
            && width <= MAX_INPUT_WIDTH
            && height <= MAX_INPUT_HEIGHT
            && u64::try_from(self.decoded_bytes_upper_bound)
                .is_ok_and(|value| value <= 64_000_000_000)
            && u64::try_from(self.frame_count_upper_bound).is_ok_and(|value| value <= 1_300_000)
            && u16::try_from(self.track_count).is_ok_and(|value| value <= 32)
            && transform_fits(transform, duration)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedExecutionSeed {
    pub profile_version: u16,
    pub normalized_profile_sha256: String,
    pub route_reason: &'static str,
    pub selected_executor: &'static str,
    pub fallback_executor: Option<&'static str>,
    pub output_content_type: &'static str,
    pub max_output_bytes: u64,
}

impl ManagedExecutionSeed {
    pub(crate) fn for_request(
        request: &MediaJobRequest,
        probe: &MediaSourceProbeRow,
        managed_enabled: bool,
    ) -> std::result::Result<Self, ManagedRuntimeError> {
        let transform = request
            .transform
            .as_ref()
            .ok_or(ManagedRuntimeError::InvalidRequest)?;
        transform
            .validate_for(&request.profile)
            .map_err(|_| ManagedRuntimeError::InvalidRequest)?;
        let (route_reason, selected_executor, fallback_executor) = if !managed_enabled {
            ("managed_kill_switch", "native_gstreamer", None)
        } else if probe.supports_managed(transform) {
            (
                "managed_preferred",
                "cloudflare_media",
                Some("native_gstreamer"),
            )
        } else {
            (rejection_reason(probe, transform), "native_gstreamer", None)
        };
        Ok(Self {
            profile_version: transform.profile_version,
            normalized_profile_sha256: normalized_profile_digest(&request.profile, transform)?,
            route_reason,
            selected_executor,
            fallback_executor,
            output_content_type: output_content_type(transform.format),
            max_output_bytes: transform.max_output_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedRuntimeError {
    InvalidRequest,
    UntrustedProbe,
    LeaseLost,
    Cancelled,
    Persistence,
    Provider(CloudflareMediaAdapterError),
}

impl From<CloudflareMediaAdapterError> for ManagedRuntimeError {
    fn from(value: CloudflareMediaAdapterError) -> Self {
        Self::Provider(value)
    }
}

impl ManagedRuntimeError {
    const fn safe_code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UntrustedProbe => "untrusted_probe",
            Self::LeaseLost => "lease_lost",
            Self::Cancelled => "cancelled",
            Self::Persistence => "persistence",
            Self::Provider(error) => error.safe_code(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExecutionRow {
    job_id: String,
    organization_id: String,
    video_id: String,
    source_version: i64,
    profile_id: String,
    normalized_profile_sha256: String,
    attempt: i64,
    lease_epoch: i64,
    lease_token_digest: String,
    staging_object_key: Option<String>,
    final_object_key: String,
    output_content_type: String,
    max_output_bytes: i64,
    staged_checksum_sha256: Option<String>,
    staged_bytes: Option<i64>,
    provider_operations: i64,
    provider_output_seconds: i64,
    payload_json: String,
    cancel_requested: i64,
    source_object_key: String,
    source_checksum_sha256: String,
    source_bytes: i64,
    source_content_type: String,
    duration_ms: i64,
}

#[derive(Debug, Deserialize)]
struct IdRow {
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct CancelRow {
    cancel_requested: i64,
}

#[derive(Debug, Deserialize)]
struct CancellationRecoveryRow {
    job_id: String,
    organization_id: String,
    state: String,
    staging_object_key: Option<String>,
    final_object_key: String,
    output_content_type: String,
    normalized_profile_sha256: String,
    source_checksum_sha256: String,
    updated_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct ExecutionScopeRow {
    organization_id: String,
}

async fn mutation_authority_fence(
    env: &Env,
    tenant_id: &str,
) -> std::result::Result<crate::MutationAuthorityFence, ManagedRuntimeError> {
    let config = crate::RuntimeConfig::from_env(env).ok_or(ManagedRuntimeError::Persistence)?;
    crate::mutation_authority_fence(env, &config, tenant_id)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .ok_or(ManagedRuntimeError::Persistence)
}

async fn execute_mutation_batch(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    operation_id: &str,
    occurred_at_ms: i64,
    statements: Vec<D1PreparedStatement>,
) -> std::result::Result<(), ManagedRuntimeError> {
    let expected = statements.len();
    let results = crate::execute_mutation_batch(
        database,
        authority_fence,
        operation_id,
        occurred_at_ms,
        statements,
    )
    .await
    .map_err(|_| ManagedRuntimeError::Persistence)?;
    if results.len() != expected || results.iter().any(|result| !result.success()) {
        return Err(ManagedRuntimeError::Persistence);
    }
    Ok(())
}

pub(crate) async fn load_verified_probe(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
    source_version: u32,
) -> Result<Option<MediaSourceProbeRow>> {
    database
        .prepare(
            "SELECT organization_id, video_id, source_version, source_object_key, \
                    source_checksum_sha256, source_bytes, source_content_type, container, \
                    video_codec, audio_codec, duration_ms, width, height, \
                    decoded_bytes_upper_bound, frame_count_upper_bound, track_count, \
                    probe_digest, trust, state FROM media_source_probes_v1 \
              WHERE organization_id = ?1 AND video_id = ?2 AND source_version = ?3 \
                AND trust = 'verified_native_probe' AND state = 'verified' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(video_id),
            JsValue::from_f64(f64::from(source_version)),
        ])?
        .first::<MediaSourceProbeRow>(None)
        .await
}

#[must_use]
pub(crate) fn managed_media_enabled(env: &Env) -> bool {
    env.var("FRAME_MANAGED_MEDIA_STATE")
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "enabled".into())
        == "enabled"
}

pub(crate) async fn process_job(env: Env, job_id: String) {
    if let Err(error) = process_job_inner(&env, &job_id).await {
        worker::console_error!("managed media execution failed class={}", error.safe_code());
    }
}

pub(crate) async fn recover_one(env: Env) {
    let result = async {
        let database = env.d1("DB").map_err(|_| ManagedRuntimeError::Persistence)?;
        let now = now_ms()?;
        recover_cancelled_one(&env, &database, now).await?;
        let candidate = database
            .prepare(
                "SELECT e.job_id FROM media_job_execution_v1 e \
                  JOIN media_jobs j ON j.id = e.job_id \
                 WHERE e.selected_executor = 'cloudflare_media' AND e.attempt < ?2 \
                   AND j.cancel_requested = 0 AND (e.state = 'queued' OR \
                     (e.state IN ('leased','transforming','staged','publishing') \
                       AND e.lease_expires_at_ms <= ?1)) \
                 ORDER BY CASE e.state WHEN 'publishing' THEN 0 WHEN 'staged' THEN 1 ELSE 2 END, \
                          e.updated_at_ms, e.job_id LIMIT 1",
            )
            .bind(&[
                JsValue::from_f64(now as f64),
                JsValue::from_f64(MAX_ATTEMPTS as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?
            .first::<IdRow>(None)
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?;
        if let Some(candidate) = candidate {
            process_job_inner(&env, &candidate.job_id).await?;
        }
        Ok::<(), ManagedRuntimeError>(())
    }
    .await;
    if let Err(error) = result {
        worker::console_error!("managed media recovery failed class={}", error.safe_code());
    }
}

async fn recover_cancelled_one(
    env: &Env,
    database: &D1Database,
    now: i64,
) -> std::result::Result<(), ManagedRuntimeError> {
    let Some(mut candidate) = database
        .prepare(
            "SELECT e.job_id, e.organization_id, e.state, e.staging_object_key, e.final_object_key, \
                    e.output_content_type, e.normalized_profile_sha256, \
                    p.source_checksum_sha256, e.updated_at_ms \
               FROM media_job_execution_v1 e \
               JOIN media_jobs j ON j.id = e.job_id \
               JOIN media_source_probes_v1 p ON p.organization_id = e.organization_id \
                AND p.video_id = e.video_id AND p.source_version = e.source_version \
              WHERE e.selected_executor = 'cloudflare_media' AND j.cancel_requested = 1 \
                AND e.state IN ('queued','leased','transforming','staged','publishing','cancelling') \
              ORDER BY CASE e.state WHEN 'cancelling' THEN 0 ELSE 1 END, e.updated_at_ms, e.job_id \
              LIMIT 1",
        )
        .first::<CancellationRecoveryRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
    else {
        return Ok(());
    };
    let authority_fence = mutation_authority_fence(env, &candidate.organization_id).await?;

    if candidate.state != "cancelling" {
        execute_mutation_batch(
            database,
            &authority_fence,
            &format!("managed-cancel-start:{}", candidate.job_id),
            now,
            vec![
                database
                    .prepare(
                        "UPDATE media_job_execution_v1 SET state = 'cancelling', \
                       failure_class = 'cancelled', lease_token_digest = NULL, \
                       lease_expires_at_ms = NULL, updated_at_ms = ?2 \
                     WHERE job_id = ?1 AND state IN \
                       ('queued','leased','transforming','staged','publishing') \
                       AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                         AND j.cancel_requested = 1)",
                    )
                    .bind(&[
                        JsValue::from_str(&candidate.job_id),
                        JsValue::from_f64(now as f64),
                    ])
                    .map_err(|_| ManagedRuntimeError::Persistence)?,
            ],
        )
        .await?;
        let transitioned = database
            .prepare(
                "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 \
                 AND state = 'cancelling' AND failure_class = 'cancelled' \
                 AND updated_at_ms = ?2 LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?
            .first::<IdRow>(None)
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?
            .is_some();
        if !transitioned {
            return Ok(());
        }
        candidate.state = "cancelling".into();
        candidate.updated_at_ms = now;
    }

    // Once the execution is fenced in `cancelling`, public state can become
    // terminal. The execution itself remains recoverable until object absence
    // has been observed after the in-flight Worker lease horizon.
    execute_mutation_batch(
        database,
        &authority_fence,
        &format!("managed-cancel-public:{}", candidate.job_id),
        now,
        vec![
            database
                .prepare(
                    "UPDATE media_jobs SET state = 'cancelled', progress_basis_points = 0, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?2, revision = revision + 1 \
                 WHERE id = ?1 AND cancel_requested = 1 \
                   AND state IN ('queued','leased','running')",
                )
                .bind(&[
                    JsValue::from_str(&candidate.job_id),
                    JsValue::from_f64(now as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;

    if let Some(staging_key) = candidate.staging_object_key.as_ref() {
        let plan = CloudflareCancellationPlan {
            staging_key: staging_key.clone(),
            final_key: candidate.final_object_key.clone(),
            source_sha256: candidate.source_checksum_sha256.clone(),
            profile_sha256: candidate.normalized_profile_sha256.clone(),
            content_type: candidate.output_content_type.clone(),
        };
        if !CloudflareMediaBindingAdapter::cancel_planned_and_confirm_absent(env, &plan).await? {
            return Err(ManagedRuntimeError::Persistence);
        }
        if now.saturating_sub(candidate.updated_at_ms) < CANCELLATION_SETTLE_MS {
            return Ok(());
        }
    }

    execute_mutation_batch(
        database,
        &authority_fence,
        &format!("managed-cancel-finish:{}", candidate.job_id),
        now,
        vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'cancelled', updated_at_ms = ?2 \
                 WHERE job_id = ?1 AND state = 'cancelling' \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 1 AND j.state = 'cancelled')",
                )
                .bind(&[
                    JsValue::from_str(&candidate.job_id),
                    JsValue::from_f64(now as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;
    let finalized = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 \
             AND state = 'cancelled' AND updated_at_ms = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&candidate.job_id),
            JsValue::from_f64(now as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .is_some();
    if !finalized {
        return Err(ManagedRuntimeError::Persistence);
    }
    Ok(())
}

async fn process_job_inner(
    env: &Env,
    job_id: &str,
) -> std::result::Result<(), ManagedRuntimeError> {
    let database = env.d1("DB").map_err(|_| ManagedRuntimeError::Persistence)?;
    let scope = database
        .prepare("SELECT organization_id FROM media_job_execution_v1 WHERE job_id = ?1 LIMIT 1")
        .bind(&[JsValue::from_str(job_id)])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<ExecutionScopeRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .ok_or(ManagedRuntimeError::Persistence)?;
    let authority_fence = mutation_authority_fence(env, &scope.organization_id).await?;
    let execution = claim(&database, &authority_fence, job_id).await?;
    if execution.organization_id != scope.organization_id {
        return Err(ManagedRuntimeError::Persistence);
    }
    let staged = if let (Some(checksum), Some(bytes), Some(staging_key)) = (
        execution.staged_checksum_sha256.clone(),
        execution.staged_bytes,
        execution.staging_object_key.clone(),
    ) {
        CloudflareStagedOutput {
            staging_key,
            final_key: execution.final_object_key.clone(),
            bytes: u64::try_from(bytes).map_err(|_| ManagedRuntimeError::Persistence)?,
            checksum_sha256: checksum,
            content_type: execution.output_content_type.clone(),
            source_sha256: execution.source_checksum_sha256.clone(),
            profile_sha256: execution.normalized_profile_sha256.clone(),
            estimated_provider_operations: u64::try_from(execution.provider_operations)
                .map_err(|_| ManagedRuntimeError::Persistence)?,
            provider_output_seconds: u64::try_from(execution.provider_output_seconds)
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        }
    } else {
        transform(env, &database, &authority_fence, &execution).await?
    };
    if cancelled(&database, &execution.job_id).await? {
        cancel_cleanup(env, &database, &authority_fence, &execution, &staged).await?;
        return Err(ManagedRuntimeError::Cancelled);
    }
    transition_to_publishing(&database, &authority_fence, &execution, &staged).await?;
    CloudflareMediaBindingAdapter::publish_staged(env, &staged).await?;
    if cancelled(&database, &execution.job_id).await? {
        cancel_cleanup(env, &database, &authority_fence, &execution, &staged).await?;
        return Err(ManagedRuntimeError::Cancelled);
    }
    match commit(&database, &authority_fence, &execution, &staged).await {
        Err(ManagedRuntimeError::Cancelled) => {
            cancel_cleanup(env, &database, &authority_fence, &execution, &staged).await?;
            Err(ManagedRuntimeError::Cancelled)
        }
        result => result,
    }
}

async fn claim(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    job_id: &str,
) -> std::result::Result<ExecutionRow, ManagedRuntimeError> {
    let now = now_ms()?;
    let lease = random_digest()?;
    let expires = now
        .checked_add(LEASE_MS)
        .ok_or(ManagedRuntimeError::Persistence)?;
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-claim:{job_id}"),
        now,
        vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'leased', \
                        attempt = CASE WHEN state = 'queued' THEN attempt + 1 ELSE attempt END, \
                        lease_epoch = lease_epoch + 1, lease_token_digest = ?2, \
                        lease_expires_at_ms = ?3, updated_at_ms = ?4 \
                  WHERE job_id = ?1 AND selected_executor = 'cloudflare_media' \
                    AND attempt < ?5 AND (state = 'queued' OR \
                      (state IN ('leased','transforming','staged','publishing') \
                        AND lease_expires_at_ms <= ?4)) \
                    AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                      AND j.cancel_requested = 0 AND j.state IN ('queued','leased','running'))",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_str(&lease),
                    JsValue::from_f64(expires as f64),
                    JsValue::from_f64(now as f64),
                    JsValue::from_f64(MAX_ATTEMPTS as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;
    let claimed = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 AND state = 'leased' \
             AND lease_token_digest = ?2 AND lease_expires_at_ms = ?3 AND updated_at_ms = ?4 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(&lease),
            JsValue::from_f64(expires as f64),
            JsValue::from_f64(now as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .ok_or(ManagedRuntimeError::LeaseLost)?;
    if claimed.job_id != job_id {
        return Err(ManagedRuntimeError::Persistence);
    }
    let row = database
        .prepare(
            "SELECT e.job_id, e.organization_id, e.video_id, e.source_version, e.profile_id, \
                    e.normalized_profile_sha256, e.attempt, e.lease_epoch, \
                    e.lease_token_digest, e.staging_object_key, e.final_object_key, \
                    e.output_content_type, e.max_output_bytes, e.staged_checksum_sha256, \
                    e.staged_bytes, e.provider_operations, e.provider_output_seconds, \
                    j.payload_json, j.cancel_requested, p.source_object_key, \
                    p.source_checksum_sha256, p.source_bytes, p.source_content_type, p.duration_ms \
               FROM media_job_execution_v1 e \
               JOIN media_jobs j ON j.id = e.job_id \
               JOIN media_source_probes_v1 p ON p.organization_id = e.organization_id \
                AND p.video_id = e.video_id AND p.source_version = e.source_version \
              WHERE e.job_id = ?1 AND e.state = 'leased' AND e.lease_token_digest = ?2 \
                AND p.trust = 'verified_native_probe' AND p.state = 'verified' LIMIT 1",
        )
        .bind(&[JsValue::from_str(job_id), JsValue::from_str(&lease)])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<ExecutionRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .ok_or(ManagedRuntimeError::Persistence)?;
    if row.lease_token_digest != lease
        || row.source_version <= 0
        || row.attempt <= 0
        || row.lease_epoch <= 0
        || row.cancel_requested != 0
        || row.max_output_bytes <= 0
        || !valid_sha256(&row.lease_token_digest)
        || !valid_sha256(&row.source_checksum_sha256)
        || !valid_sha256(&row.normalized_profile_sha256)
    {
        return Err(ManagedRuntimeError::Persistence);
    }
    Ok(row)
}

async fn transform(
    env: &Env,
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    execution: &ExecutionRow,
) -> std::result::Result<CloudflareStagedOutput, ManagedRuntimeError> {
    let body: MediaJobRequest = serde_json::from_str(&execution.payload_json)
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    let profile = body
        .transform
        .as_ref()
        .ok_or(ManagedRuntimeError::Persistence)?;
    let attempt = u16::try_from(execution.attempt).map_err(|_| ManagedRuntimeError::Persistence)?;
    let staging_key = execution
        .staging_object_key
        .clone()
        .unwrap_or_else(|| format!("{}.attempt-{attempt}.partial", execution.final_object_key));
    let request = CloudflareMediaRequest {
        tenant_id: execution.organization_id.clone(),
        video_id: execution.video_id.clone(),
        source_key: execution.source_object_key.clone(),
        source_bytes: u64::try_from(execution.source_bytes)
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        source_sha256: execution.source_checksum_sha256.clone(),
        source_content_type: execution.source_content_type.clone(),
        source_duration_ms: u64::try_from(execution.duration_ms)
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        profile_sha256: execution.normalized_profile_sha256.clone(),
        staging_key: staging_key.clone(),
        final_key: execution.final_object_key.clone(),
        mode: binding_mode(profile.mode),
        format: binding_format(profile.format),
        start_ms: profile.start_ms,
        duration_ms: profile.duration_ms,
        width: profile.width,
        height: profile.height,
        fit: binding_fit(profile.fit),
        image_count: profile.image_count,
        include_audio: profile.include_audio,
        max_output_bytes: profile.max_output_bytes,
    };
    let transforming_at = now_ms()?;
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-transform-start:{}", execution.job_id),
        transforming_at,
        vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'transforming', \
                   staging_object_key = ?5, updated_at_ms = ?6 \
                 WHERE job_id = ?1 AND state = 'leased' AND attempt = ?2 \
                   AND lease_epoch = ?3 AND lease_token_digest = ?4 \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 0)",
                )
                .bind(&[
                    JsValue::from_str(&execution.job_id),
                    JsValue::from_f64(execution.attempt as f64),
                    JsValue::from_f64(execution.lease_epoch as f64),
                    JsValue::from_str(&execution.lease_token_digest),
                    JsValue::from_str(&staging_key),
                    JsValue::from_f64(transforming_at as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;
    let transitioned = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 \
             AND state = 'transforming' AND attempt = ?2 AND lease_epoch = ?3 \
             AND lease_token_digest = ?4 AND staging_object_key = ?5 \
             AND updated_at_ms = ?6 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_f64(execution.attempt as f64),
            JsValue::from_f64(execution.lease_epoch as f64),
            JsValue::from_str(&execution.lease_token_digest),
            JsValue::from_str(&staging_key),
            JsValue::from_f64(transforming_at as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .is_some();
    if !transitioned {
        return Err(ManagedRuntimeError::LeaseLost);
    }
    let staged = match CloudflareMediaBindingAdapter::execute_to_staging(env, &request).await {
        Ok(value) => value,
        Err(error) => {
            record_failure(database, authority_fence, execution, error).await?;
            return Err(ManagedRuntimeError::Provider(error));
        }
    };
    let staged_at = now_ms()?;
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-transform-stage:{}", execution.job_id),
        staged_at,
        vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'staged', \
                   staged_checksum_sha256 = ?5, staged_bytes = ?6, \
                   provider_operations = provider_operations + ?7, \
                   provider_output_seconds = provider_output_seconds + ?8, updated_at_ms = ?9 \
                 WHERE job_id = ?1 AND state = 'transforming' AND attempt = ?2 \
                   AND lease_epoch = ?3 AND lease_token_digest = ?4 \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 0)",
                )
                .bind(&[
                    JsValue::from_str(&execution.job_id),
                    JsValue::from_f64(execution.attempt as f64),
                    JsValue::from_f64(execution.lease_epoch as f64),
                    JsValue::from_str(&execution.lease_token_digest),
                    JsValue::from_str(&staged.checksum_sha256),
                    JsValue::from_f64(staged.bytes as f64),
                    JsValue::from_f64(staged.estimated_provider_operations as f64),
                    JsValue::from_f64(staged.provider_output_seconds as f64),
                    JsValue::from_f64(staged_at as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;
    let recorded = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 AND state = 'staged' \
             AND attempt = ?2 AND lease_epoch = ?3 AND lease_token_digest = ?4 \
             AND staged_checksum_sha256 = ?5 AND staged_bytes = ?6 \
             AND updated_at_ms = ?7 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_f64(execution.attempt as f64),
            JsValue::from_f64(execution.lease_epoch as f64),
            JsValue::from_str(&execution.lease_token_digest),
            JsValue::from_str(&staged.checksum_sha256),
            JsValue::from_f64(staged.bytes as f64),
            JsValue::from_f64(staged_at as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .is_some();
    if !recorded {
        cancel_cleanup(env, database, authority_fence, execution, &staged).await?;
        return Err(ManagedRuntimeError::LeaseLost);
    }
    Ok(staged)
}

async fn transition_to_publishing(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    execution: &ExecutionRow,
    staged: &CloudflareStagedOutput,
) -> std::result::Result<(), ManagedRuntimeError> {
    let now = now_ms()?;
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-publish-start:{}", execution.job_id),
        now,
        vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'publishing', updated_at_ms = ?7 \
                 WHERE job_id = ?1 AND state IN ('leased','staged') AND attempt = ?2 \
                   AND lease_epoch = ?3 AND lease_token_digest = ?4 \
                   AND staged_checksum_sha256 = ?5 AND staged_bytes = ?6 \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 0)",
                )
                .bind(&[
                    JsValue::from_str(&execution.job_id),
                    JsValue::from_f64(execution.attempt as f64),
                    JsValue::from_f64(execution.lease_epoch as f64),
                    JsValue::from_str(&execution.lease_token_digest),
                    JsValue::from_str(&staged.checksum_sha256),
                    JsValue::from_f64(staged.bytes as f64),
                    JsValue::from_f64(now as f64),
                ])
                .map_err(|_| ManagedRuntimeError::Persistence)?,
        ],
    )
    .await?;
    let updated = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 \
             AND state = 'publishing' AND attempt = ?2 AND lease_epoch = ?3 \
             AND lease_token_digest = ?4 AND staged_checksum_sha256 = ?5 \
             AND staged_bytes = ?6 AND updated_at_ms = ?7 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_f64(execution.attempt as f64),
            JsValue::from_f64(execution.lease_epoch as f64),
            JsValue::from_str(&execution.lease_token_digest),
            JsValue::from_str(&staged.checksum_sha256),
            JsValue::from_f64(staged.bytes as f64),
            JsValue::from_f64(now as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    if updated.is_none() {
        return Err(ManagedRuntimeError::LeaseLost);
    }
    Ok(())
}

async fn record_failure(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    execution: &ExecutionRow,
    error: CloudflareMediaAdapterError,
) -> std::result::Result<(), ManagedRuntimeError> {
    let fallback = error.allows_native_fallback();
    let executor = if fallback {
        "native_gstreamer"
    } else {
        "cloudflare_media"
    };
    let state = if fallback {
        "fallback_queued"
    } else {
        "failed"
    };
    let public_state = if fallback { "queued" } else { "failed" };
    let now = now_ms()?;
    let statements = vec![
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = ?5, selected_executor = ?6, \
                   fallback_executor = NULL, failure_class = ?7, lease_token_digest = NULL, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?8 \
                 WHERE job_id = ?1 AND attempt = ?2 AND lease_epoch = ?3 \
                   AND lease_token_digest = ?4",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(execution.attempt as f64),
                JsValue::from_f64(execution.lease_epoch as f64),
                JsValue::from_str(&execution.lease_token_digest),
                JsValue::from_str(state),
                JsValue::from_str(executor),
                JsValue::from_str(error.failure_class()),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "UPDATE media_jobs SET selected_executor = ?2, state = ?3, error_class = ?4, \
                   attempt = CASE WHEN attempt < (SELECT e.attempt FROM media_job_execution_v1 e \
                     WHERE e.job_id = id) THEN (SELECT e.attempt FROM media_job_execution_v1 e \
                     WHERE e.job_id = id) ELSE attempt END, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?5, revision = revision + 1 \
                 WHERE id = ?1 AND state IN ('queued','leased','running') \
                   AND EXISTS (SELECT 1 FROM media_job_execution_v1 e \
                     WHERE e.job_id = id AND e.state = ?6 AND e.selected_executor = ?2 \
                       AND e.failure_class = ?4 AND e.updated_at_ms = ?5)",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_str(executor),
                JsValue::from_str(public_state),
                JsValue::from_str(error.failure_class()),
                JsValue::from_f64(now as f64),
                JsValue::from_str(state),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
    ];
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-failure:{}", execution.job_id),
        now,
        statements,
    )
    .await?;
    let consistent = database
        .prepare(
            "SELECT e.job_id FROM media_job_execution_v1 e JOIN media_jobs j ON j.id = e.job_id \
             WHERE e.job_id = ?1 AND e.state = ?2 AND e.selected_executor = ?3 \
               AND e.failure_class = ?4 AND e.updated_at_ms = ?5 \
               AND j.state = ?6 AND j.selected_executor = ?3 AND j.error_class = ?4 \
               AND j.updated_at_ms = ?5 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_str(state),
            JsValue::from_str(executor),
            JsValue::from_str(error.failure_class()),
            JsValue::from_f64(now as f64),
            JsValue::from_str(public_state),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    if consistent.is_none() {
        return Err(ManagedRuntimeError::LeaseLost);
    }
    Ok(())
}

async fn commit(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    execution: &ExecutionRow,
    staged: &CloudflareStagedOutput,
) -> std::result::Result<(), ManagedRuntimeError> {
    let now = now_ms()?;
    let role = output_role(&execution.profile_id)?;
    let manifest_json = serde_json::json!({
        "schema_version": 1,
        "job_id": execution.job_id,
        "executor": "cloudflare_media",
        "source_checksum_sha256": execution.source_checksum_sha256,
        "normalized_profile_sha256": execution.normalized_profile_sha256,
        "object_key": staged.final_key,
        "object_checksum_sha256": staged.checksum_sha256,
        "bytes": staged.bytes,
        "content_type": staged.content_type,
    })
    .to_string();
    let manifest_digest = hex(&Sha256::digest(manifest_json.as_bytes()));
    let statements = vec![
        database
            .prepare(
                "INSERT INTO object_manifests(object_key, video_id, role, bytes, checksum_sha256, \
                   content_type, created_at_ms, organization_id, object_version, state, updated_at_ms) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 'available', ?7 \
                  WHERE EXISTS (SELECT 1 FROM media_job_execution_v1 e JOIN media_jobs j \
                    ON j.id = e.job_id WHERE e.job_id = ?9 AND e.state = 'publishing' \
                    AND e.lease_epoch = ?10 AND e.lease_token_digest = ?11 \
                    AND j.cancel_requested = 0) ON CONFLICT(object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&staged.final_key),
                JsValue::from_str(&execution.video_id),
                JsValue::from_str(role),
                JsValue::from_f64(staged.bytes as f64),
                JsValue::from_str(&staged.checksum_sha256),
                JsValue::from_str(&staged.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&execution.organization_id),
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(execution.lease_epoch as f64),
                JsValue::from_str(&execution.lease_token_digest),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id, object_key, role, \
                   visibility, state, malware_disposition, immutable_revision, cache_generation, \
                   checksum_sha256, bytes, content_type, retention_until_ms, created_at_ms, updated_at_ms) \
                 SELECT ?1, ?2, ?3, 'private', 'active', 'clean', 1, 1, ?4, ?5, ?6, NULL, ?7, ?7 \
                  WHERE EXISTS (SELECT 1 FROM media_job_execution_v1 e JOIN media_jobs j \
                    ON j.id = e.job_id WHERE e.job_id = ?8 AND e.state = 'publishing' \
                    AND e.lease_epoch = ?9 AND e.lease_token_digest = ?10 \
                    AND j.cancel_requested = 0) \
                 ON CONFLICT(organization_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&execution.organization_id),
                JsValue::from_str(&staged.final_key),
                JsValue::from_str(role),
                JsValue::from_str(&staged.checksum_sha256),
                JsValue::from_f64(staged.bytes as f64),
                JsValue::from_str(&staged.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(execution.lease_epoch as f64),
                JsValue::from_str(&execution.lease_token_digest),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "INSERT INTO media_output_manifests_v1(manifest_digest, job_id, organization_id, \
                   video_id, executor, source_checksum_sha256, normalized_profile_sha256, object_key, \
                   object_checksum_sha256, bytes, content_type, manifest_json, created_at_ms) \
                 SELECT ?1, ?2, ?3, ?4, 'cloudflare_media', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12 \
                  WHERE EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?2 AND j.cancel_requested = 0) \
                 ON CONFLICT(job_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&manifest_digest),
                JsValue::from_str(&execution.job_id),
                JsValue::from_str(&execution.organization_id),
                JsValue::from_str(&execution.video_id),
                JsValue::from_str(&execution.source_checksum_sha256),
                JsValue::from_str(&execution.normalized_profile_sha256),
                JsValue::from_str(&staged.final_key),
                JsValue::from_str(&staged.checksum_sha256),
                JsValue::from_f64(staged.bytes as f64),
                JsValue::from_str(&staged.content_type),
                JsValue::from_str(&manifest_json),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = 'succeeded', manifest_digest = ?5, \
                   lease_token_digest = NULL, lease_expires_at_ms = NULL, updated_at_ms = ?6 \
                 WHERE job_id = ?1 AND state = 'publishing' AND lease_epoch = ?2 \
                   AND lease_token_digest = ?3 AND staged_checksum_sha256 = ?4 \
                   AND EXISTS (SELECT 1 FROM media_output_manifests_v1 m \
                     WHERE m.job_id = job_id AND m.manifest_digest = ?5)",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(execution.lease_epoch as f64),
                JsValue::from_str(&execution.lease_token_digest),
                JsValue::from_str(&staged.checksum_sha256),
                JsValue::from_str(&manifest_digest),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'succeeded', progress_basis_points = 10000, \
                   usage_units = ?2, updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND cancel_requested = 0 AND EXISTS (SELECT 1 \
                   FROM media_job_execution_v1 e WHERE e.job_id = id AND e.state = 'succeeded')",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(staged.estimated_provider_operations as f64),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
    ];
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-commit:{}", execution.job_id),
        now,
        statements,
    )
    .await?;
    let committed = database
        .prepare(
            "SELECT job_id FROM media_job_execution_v1 WHERE job_id = ?1 \
               AND state = 'succeeded' AND manifest_digest = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_str(&manifest_digest),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    if committed.is_none() {
        return Err(ManagedRuntimeError::Cancelled);
    }
    Ok(())
}

async fn cancel_cleanup(
    env: &Env,
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    execution: &ExecutionRow,
    staged: &CloudflareStagedOutput,
) -> std::result::Result<(), ManagedRuntimeError> {
    if !CloudflareMediaBindingAdapter::cancel_and_confirm_absent(env, staged).await? {
        return Err(ManagedRuntimeError::Persistence);
    }
    let now = now_ms()?;
    let statements = vec![
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = 'cancelled', \
                   failure_class = 'cancelled', lease_token_digest = NULL, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?2 \
                 WHERE job_id = ?1 AND state NOT IN \
                   ('succeeded','failed','cancelled','dead_letter')",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'cancelled', progress_basis_points = 0, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?2, revision = revision + 1 \
                 WHERE id = ?1 AND cancel_requested = 1 \
                   AND state IN ('queued','leased','running') \
                   AND EXISTS (SELECT 1 FROM media_job_execution_v1 e \
                     WHERE e.job_id = id AND e.state = 'cancelled' \
                       AND e.failure_class = 'cancelled' AND e.updated_at_ms = ?2)",
            )
            .bind(&[
                JsValue::from_str(&execution.job_id),
                JsValue::from_f64(now as f64),
            ])
            .map_err(|_| ManagedRuntimeError::Persistence)?,
    ];
    execute_mutation_batch(
        database,
        authority_fence,
        &format!("managed-cancel-cleanup:{}", execution.job_id),
        now,
        statements,
    )
    .await?;
    let consistent = database
        .prepare(
            "SELECT e.job_id FROM media_job_execution_v1 e JOIN media_jobs j ON j.id = e.job_id \
             WHERE e.job_id = ?1 AND e.state = 'cancelled' AND e.failure_class = 'cancelled' \
               AND e.updated_at_ms = ?2 AND j.state = 'cancelled' \
               AND j.cancel_requested = 1 AND j.updated_at_ms = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&execution.job_id),
            JsValue::from_f64(now as f64),
        ])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<IdRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    if consistent.is_none() {
        return Err(ManagedRuntimeError::LeaseLost);
    }
    Ok(())
}

async fn cancelled(
    database: &D1Database,
    job_id: &str,
) -> std::result::Result<bool, ManagedRuntimeError> {
    database
        .prepare("SELECT cancel_requested FROM media_jobs WHERE id = ?1 LIMIT 1")
        .bind(&[JsValue::from_str(job_id)])
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .first::<CancelRow>(None)
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?
        .map(|row| row.cancel_requested == 1)
        .ok_or(ManagedRuntimeError::Persistence)
}

fn transform_fits(profile: &MediaTransformRequest, source_duration: u64) -> bool {
    profile.start_ms <= MAX_INPUT_DURATION_MS
        && profile.start_ms < source_duration
        && profile.start_ms.is_multiple_of(1_000)
        && profile.max_output_bytes <= MAX_OUTPUT_BYTES
        && profile
            .width
            .is_none_or(|value| (MIN_OUTPUT_DIMENSION..=MAX_OUTPUT_DIMENSION).contains(&value))
        && profile
            .height
            .is_none_or(|value| (MIN_OUTPUT_DIMENSION..=MAX_OUTPUT_DIMENSION).contains(&value))
        && profile
            .image_count
            .is_none_or(|value| value <= MAX_IMAGE_COUNT)
        && profile.duration_ms.is_none_or(|duration| {
            (MIN_OUTPUT_DURATION_MS..=MAX_OUTPUT_DURATION_MS).contains(&duration)
                && duration.is_multiple_of(1_000)
                && profile.start_ms.saturating_add(duration) <= source_duration
        })
}

fn rejection_reason(probe: &MediaSourceProbeRow, profile: &MediaTransformRequest) -> &'static str {
    if probe.source_content_type != "video/mp4"
        || probe.container != "mp4"
        || probe.video_codec != "h264"
        || !matches!(probe.audio_codec.as_str(), "aac" | "mp3" | "none")
    {
        "managed_input_format"
    } else if probe.source_bytes >= MAX_INPUT_BYTES_EXCLUSIVE as i64
        || probe.duration_ms > MAX_INPUT_DURATION_MS as i64
        || probe.width > i64::from(MAX_INPUT_WIDTH)
        || probe.height > i64::from(MAX_INPUT_HEIGHT)
    {
        "managed_input_limit"
    } else if !transform_fits(profile, u64::try_from(probe.duration_ms).unwrap_or(0)) {
        "managed_profile_limit"
    } else {
        "managed_input_limit"
    }
}

fn normalized_profile_digest(
    profile_id: &str,
    profile: &MediaTransformRequest,
) -> std::result::Result<String, ManagedRuntimeError> {
    let encoded = serde_json::to_vec(&(profile_id, profile))
        .map_err(|_| ManagedRuntimeError::InvalidRequest)?;
    let mut digest = Sha256::new();
    digest.update(b"frame-media-profile-v1\0");
    digest.update(encoded);
    Ok(hex(&digest.finalize()))
}

const fn output_content_type(format: ManagedMediaFormat) -> &'static str {
    match format {
        ManagedMediaFormat::Mp4H264Aac => "video/mp4",
        ManagedMediaFormat::Jpeg => "image/jpeg",
        ManagedMediaFormat::Png => "image/png",
        ManagedMediaFormat::M4aAac => "audio/mp4",
    }
}

const fn binding_mode(mode: ManagedMediaMode) -> CloudflareMediaMode {
    match mode {
        ManagedMediaMode::Video => CloudflareMediaMode::Video,
        ManagedMediaMode::Frame => CloudflareMediaMode::Frame,
        ManagedMediaMode::Spritesheet => CloudflareMediaMode::Spritesheet,
        ManagedMediaMode::Audio => CloudflareMediaMode::Audio,
    }
}

const fn binding_format(format: ManagedMediaFormat) -> CloudflareMediaFormat {
    match format {
        ManagedMediaFormat::Mp4H264Aac => CloudflareMediaFormat::Mp4H264Aac,
        ManagedMediaFormat::Jpeg => CloudflareMediaFormat::Jpeg,
        ManagedMediaFormat::Png => CloudflareMediaFormat::Png,
        ManagedMediaFormat::M4aAac => CloudflareMediaFormat::M4aAac,
    }
}

const fn binding_fit(fit: MediaResizeFit) -> &'static str {
    match fit {
        MediaResizeFit::Contain => "contain",
        MediaResizeFit::Cover => "cover",
        MediaResizeFit::ScaleDown => "scale-down",
    }
}

fn output_role(profile_id: &str) -> std::result::Result<&'static str, ManagedRuntimeError> {
    match profile_id {
        "optimized_clip_v1" => Ok("preview"),
        "thumbnail_v1" => Ok("thumbnail"),
        "spritesheet_v1" => Ok("spritesheet"),
        "audio_extract_v1" => Ok("audio"),
        _ => Err(ManagedRuntimeError::Persistence),
    }
}

fn random_digest() -> std::result::Result<String, ManagedRuntimeError> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| ManagedRuntimeError::Persistence)?;
    Ok(hex(&Sha256::digest(random)))
}

fn now_ms() -> std::result::Result<i64, ManagedRuntimeError> {
    let value = js_sys::Date::now();
    if !value.is_finite() || !(0.0..=9_007_199_254_740_991.0).contains(&value) {
        return Err(ManagedRuntimeError::Persistence);
    }
    Ok(value as i64)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> MediaTransformRequest {
        MediaTransformRequest {
            schema_version: 1,
            profile_version: 1,
            mode: ManagedMediaMode::Video,
            start_ms: 0,
            duration_ms: Some(5_000),
            width: Some(640),
            height: Some(360),
            fit: MediaResizeFit::Contain,
            image_count: None,
            include_audio: true,
            format: ManagedMediaFormat::Mp4H264Aac,
            max_output_bytes: 8_000_000,
        }
    }

    fn probe() -> MediaSourceProbeRow {
        MediaSourceProbeRow {
            organization_id: "018f0b5f-1f52-7c2d-8c50-2c2dc5f2a101".into(),
            video_id: "018f0b5f-1f52-7c2d-8c50-2c2dc5f2a102".into(),
            source_version: 1,
            source_object_key: "tenants/tenant/videos/video/source/v1/input.mp4".into(),
            source_checksum_sha256: "a".repeat(64),
            source_bytes: 1_000_000,
            source_content_type: "video/mp4".into(),
            container: "mp4".into(),
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_ms: 120_000,
            width: 1920,
            height: 1080,
            decoded_bytes_upper_bound: 12_000_000_000,
            frame_count_upper_bound: 3_600,
            track_count: 2,
            probe_digest: "b".repeat(64),
            trust: "verified_native_probe".into(),
            state: "verified".into(),
        }
    }

    #[test]
    fn exact_managed_boundary_and_one_over_are_deterministic() {
        let mut probe = probe();
        probe.source_bytes = MAX_INPUT_BYTES_EXCLUSIVE as i64 - 1;
        assert!(probe.supports_managed(&profile()));
        probe.source_bytes += 1;
        assert!(!probe.supports_managed(&profile()));
        assert_eq!(rejection_reason(&probe, &profile()), "managed_input_limit");
    }

    #[test]
    fn normalized_identity_binds_material_transform_fields() {
        let first = normalized_profile_digest("optimized_clip_v1", &profile()).expect("digest");
        let mut changed = profile();
        changed.width = Some(1280);
        changed.height = Some(720);
        let second = normalized_profile_digest("optimized_clip_v1", &changed).expect("digest");
        assert_ne!(first, second);
        assert!(valid_sha256(&first));
    }

    #[test]
    fn native_probe_is_canonical_and_bounds_are_server_derived() {
        let manifest = ProbeManifestV1 {
            schema_version: 1,
            profile: "probe_v1".into(),
            container: "mp4".into(),
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_ms: 2_000,
            width: 640,
            height: 360,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            track_count: 2,
        };
        let canonical = serde_json::to_vec(&manifest).expect("canonical manifest");
        let verified = verify_native_probe_v1(&canonical, "video/mp4").expect("valid probe");
        assert_eq!(verified.frame_count_upper_bound, 60);
        assert_eq!(verified.decoded_bytes_upper_bound, 67_584_000);

        let mut padded = canonical.clone();
        padded.push(b'\n');
        assert_eq!(
            verify_native_probe_v1(&padded, "video/mp4"),
            Err(ManagedRuntimeError::InvalidRequest)
        );
        assert_eq!(
            verify_native_probe_v1(&canonical, "video/webm"),
            Err(ManagedRuntimeError::InvalidRequest)
        );
    }

    #[test]
    fn native_probe_rejects_bomb_bounds_and_inconsistent_tracks() {
        let mut manifest = ProbeManifestV1 {
            schema_version: 1,
            profile: "probe_v1".into(),
            container: "webm".into(),
            video_codec: "vp9".into(),
            audio_codec: "opus".into(),
            duration_ms: 2_000,
            width: 640,
            height: 360,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            track_count: 1,
        };
        let bytes = serde_json::to_vec(&manifest).expect("manifest");
        assert_eq!(
            verify_native_probe_v1(&bytes, "video/webm"),
            Err(ManagedRuntimeError::InvalidRequest)
        );
        manifest.track_count = 2;
        manifest.width = MAX_NATIVE_PROBE_WIDTH;
        manifest.height = MAX_NATIVE_PROBE_HEIGHT;
        manifest.duration_ms = MAX_NATIVE_PROBE_DURATION_MS;
        manifest.frame_rate_numerator = MAX_PROBE_FRAME_RATE as u32;
        let bytes = serde_json::to_vec(&manifest).expect("bomb manifest");
        assert_eq!(
            verify_native_probe_v1(&bytes, "video/webm"),
            Err(ManagedRuntimeError::InvalidRequest)
        );
    }
}
