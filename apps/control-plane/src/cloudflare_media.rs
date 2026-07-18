//! Isolated interop for the Cloudflare Media Transformations Workers binding.
//!
//! The binding has no first-class `workers-rs` 0.8.5 wrapper. All dynamic JS
//! calls stay in this module; callers exchange bounded, value-free Rust types.

use std::collections::HashMap;

use futures::{StreamExt, TryStreamExt};
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue};
use worker::wasm_bindgen_futures::JsFuture;
use worker::web_sys::ReadableStream;
use worker::{ByteStream, Conditional, Env, FixedLengthStream, HttpMetadata};

const MEDIA_BINDING: &str = "MEDIA";
const RECORDINGS_BINDING: &str = "RECORDINGS";
const MAX_INPUT_BYTES_EXCLUSIVE: u64 = 100_000_000;
const MAX_INPUT_DURATION_MS: u64 = 600_000;
const MAX_OUTPUT_DURATION_MS: u64 = 60_000;
const MIN_OUTPUT_DURATION_MS: u64 = 1_000;
const MAX_START_MS: u64 = 600_000;
const MIN_DIMENSION: u32 = 10;
const MAX_DIMENSION: u32 = 2_000;
const MAX_IMAGE_COUNT: u16 = 100;
// The binding result stream is consumed incrementally, but the current R2 SHA-256
// API requires a known checksum before the conditional PUT. This operator cap
// keeps the one bounded staging buffer below Worker memory limits.
const MAX_BUFFERED_OUTPUT_BYTES: u64 = 32_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudflareMediaMode {
    Video,
    Frame,
    Spritesheet,
    Audio,
}

impl CloudflareMediaMode {
    const fn as_binding_value(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Frame => "frame",
            Self::Spritesheet => "spritesheet",
            Self::Audio => "audio",
        }
    }

    #[cfg(test)]
    const fn implementation_id(self) -> &'static str {
        match self {
            Self::Video => "cloudflare_video_bounded_v1",
            Self::Frame => "cloudflare_frame_bounded_v1",
            Self::Spritesheet => "cloudflare_spritesheet_bounded_v1",
            Self::Audio => "cloudflare_audio_bounded_v1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudflareMediaFormat {
    Mp4H264Aac,
    Jpeg,
    Png,
    M4aAac,
}

impl CloudflareMediaFormat {
    pub(crate) const fn content_type(self) -> &'static str {
        match self {
            Self::Mp4H264Aac => "video/mp4",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::M4aAac => "audio/mp4",
        }
    }

    const fn binding_value(self) -> Option<&'static str> {
        match self {
            Self::Jpeg => Some("jpg"),
            Self::Png => Some("png"),
            Self::M4aAac => Some("m4a"),
            Self::Mp4H264Aac => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CloudflareMediaRequest {
    pub tenant_id: String,
    pub video_id: String,
    pub source_key: String,
    pub source_bytes: u64,
    pub source_sha256: String,
    pub source_content_type: String,
    pub source_duration_ms: u64,
    pub profile_sha256: String,
    pub staging_key: String,
    pub final_key: String,
    pub mode: CloudflareMediaMode,
    pub format: CloudflareMediaFormat,
    pub start_ms: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: &'static str,
    pub image_count: Option<u16>,
    pub include_audio: bool,
    pub max_output_bytes: u64,
}

impl std::fmt::Debug for CloudflareMediaRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudflareMediaRequest")
            .field("tenant_id", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("source_key", &"[redacted]")
            .field("source_bytes", &self.source_bytes)
            .field("source_sha256", &"[redacted]")
            .field("source_content_type", &self.source_content_type)
            .field("source_duration_ms", &self.source_duration_ms)
            .field("profile_sha256", &"[redacted]")
            .field("staging_key", &"[redacted]")
            .field("final_key", &"[redacted]")
            .field("mode", &self.mode)
            .field("format", &self.format)
            .field("start_ms", &self.start_ms)
            .field("duration_ms", &self.duration_ms)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fit", &self.fit)
            .field("image_count", &self.image_count)
            .field("include_audio", &self.include_audio)
            .field("max_output_bytes", &self.max_output_bytes)
            .finish()
    }
}

impl CloudflareMediaRequest {
    pub(crate) fn validate(&self) -> Result<(), CloudflareMediaAdapterError> {
        let tenant = canonical_uuid(&self.tenant_id);
        let video = canonical_uuid(&self.video_id);
        let source_prefix = format!("tenants/{}/videos/{}/", self.tenant_id, self.video_id);
        let derivative_prefix = format!(
            "tenants/{}/videos/{}/derivatives/",
            self.tenant_id, self.video_id
        );
        if !tenant
            || !video
            || !safe_private_key(&self.source_key)
            || !safe_private_key(&self.staging_key)
            || !safe_private_key(&self.final_key)
            || !self.source_key.starts_with(&source_prefix)
            || !self.staging_key.starts_with(&derivative_prefix)
            || !self.final_key.starts_with(&derivative_prefix)
            || !valid_staging_key(&self.staging_key, &self.final_key)
            || self.final_key.ends_with(".partial")
            || self.source_bytes == 0
            || self.source_bytes >= MAX_INPUT_BYTES_EXCLUSIVE
            || self.source_duration_ms == 0
            || self.source_duration_ms > MAX_INPUT_DURATION_MS
            || self.source_content_type != "video/mp4"
            || !valid_sha256(&self.source_sha256)
            || !valid_sha256(&self.profile_sha256)
            || self.start_ms > MAX_START_MS
            || self.start_ms >= self.source_duration_ms
            || !self.start_ms.is_multiple_of(1_000)
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_BUFFERED_OUTPUT_BYTES
            || self.width.is_some() != self.height.is_some()
            || !matches!(self.fit, "contain" | "cover" | "scale-down")
        {
            return Err(CloudflareMediaAdapterError::InvalidRequest);
        }
        if self
            .width
            .is_some_and(|value| !(MIN_DIMENSION..=MAX_DIMENSION).contains(&value))
            || self
                .height
                .is_some_and(|value| !(MIN_DIMENSION..=MAX_DIMENSION).contains(&value))
        {
            return Err(CloudflareMediaAdapterError::InvalidRequest);
        }
        let timed = self.duration_ms.is_some_and(|duration| {
            (MIN_OUTPUT_DURATION_MS..=MAX_OUTPUT_DURATION_MS).contains(&duration)
                && duration.is_multiple_of(1_000)
                && self.start_ms.saturating_add(duration) <= self.source_duration_ms
        });
        let mode_valid = match self.mode {
            CloudflareMediaMode::Video => {
                self.format == CloudflareMediaFormat::Mp4H264Aac
                    && timed
                    && self.image_count.is_none()
            }
            CloudflareMediaMode::Frame => {
                matches!(
                    self.format,
                    CloudflareMediaFormat::Jpeg | CloudflareMediaFormat::Png
                ) && self.duration_ms.is_none()
                    && self.image_count.is_none()
                    && !self.include_audio
            }
            CloudflareMediaMode::Spritesheet => {
                self.format == CloudflareMediaFormat::Jpeg
                    && timed
                    && self
                        .image_count
                        .is_some_and(|count| (1..=MAX_IMAGE_COUNT).contains(&count))
                    && !self.include_audio
            }
            CloudflareMediaMode::Audio => {
                self.format == CloudflareMediaFormat::M4aAac
                    && timed
                    && self.width.is_none()
                    && self.image_count.is_none()
                    && !self.include_audio
            }
        };
        if !mode_valid {
            return Err(CloudflareMediaAdapterError::InvalidRequest);
        }
        Ok(())
    }

    const fn estimated_provider_operations(&self) -> u64 {
        match self.mode {
            CloudflareMediaMode::Frame => 1,
            CloudflareMediaMode::Spritesheet => match self.image_count {
                Some(count) => count as u64,
                None => 0,
            },
            CloudflareMediaMode::Video | CloudflareMediaMode::Audio => match self.duration_ms {
                Some(duration) => duration.div_ceil(1_000),
                None => 0,
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CloudflareStagedOutput {
    pub staging_key: String,
    pub final_key: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
    pub source_sha256: String,
    pub profile_sha256: String,
    pub estimated_provider_operations: u64,
    pub provider_output_seconds: u64,
}

/// The minimum immutable identity needed to reclaim a managed artifact after
/// the Worker died before it could persist the staged checksum. Keys and
/// metadata are still verified before a final object is removed; an object
/// that merely collides at the deterministic key is never deleted.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CloudflareCancellationPlan {
    pub staging_key: String,
    pub final_key: String,
    pub source_sha256: String,
    pub profile_sha256: String,
    pub content_type: String,
}

impl CloudflareCancellationPlan {
    fn validate(&self) -> Result<(), CloudflareMediaAdapterError> {
        if !safe_private_key(&self.staging_key)
            || !safe_private_key(&self.final_key)
            || !valid_staging_key(&self.staging_key, &self.final_key)
            || self.final_key.ends_with(".partial")
            || !valid_sha256(&self.source_sha256)
            || !valid_sha256(&self.profile_sha256)
            || !matches!(
                self.content_type.as_str(),
                "video/mp4" | "image/jpeg" | "image/png" | "audio/mp4"
            )
        {
            return Err(CloudflareMediaAdapterError::StorageConflict);
        }
        Ok(())
    }
}

impl std::fmt::Debug for CloudflareStagedOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudflareStagedOutput")
            .field("staging_key", &"[redacted]")
            .field("final_key", &"[redacted]")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .field("source_sha256", &"[redacted]")
            .field("profile_sha256", &"[redacted]")
            .field(
                "estimated_provider_operations",
                &self.estimated_provider_operations,
            )
            .field("provider_output_seconds", &self.provider_output_seconds)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudflareMediaAdapterError {
    InvalidRequest,
    BindingUnavailable,
    SourceMissing,
    SourceConflict,
    ProviderRejected { code: Option<u32> },
    OutputTooLarge,
    OutputIncompatible,
    StorageConflict,
    StorageFailure,
}

impl CloudflareMediaAdapterError {
    pub(crate) const fn safe_code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::BindingUnavailable => "binding_unavailable",
            Self::SourceMissing => "source_missing",
            Self::SourceConflict => "source_conflict",
            Self::ProviderRejected { .. } => "provider_rejected",
            Self::OutputTooLarge => "output_too_large",
            Self::OutputIncompatible => "output_incompatible",
            Self::StorageConflict => "storage_conflict",
            Self::StorageFailure => "storage_failure",
        }
    }

    pub(crate) const fn failure_class(self) -> &'static str {
        match self {
            Self::BindingUnavailable | Self::ProviderRejected { .. } => "provider_outage",
            Self::OutputTooLarge => "resource_limit",
            Self::OutputIncompatible => "output_incompatible",
            Self::InvalidRequest | Self::SourceMissing | Self::SourceConflict => "invalid_input",
            Self::StorageConflict | Self::StorageFailure => "storage_failure",
        }
    }

    pub(crate) const fn allows_native_fallback(self) -> bool {
        matches!(
            self,
            Self::BindingUnavailable
                | Self::ProviderRejected { .. }
                | Self::OutputTooLarge
                | Self::OutputIncompatible
        )
    }
}

impl std::fmt::Display for CloudflareMediaAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "managed media request is invalid",
            Self::BindingUnavailable => "managed media binding is unavailable",
            Self::SourceMissing => "managed media source is unavailable",
            Self::SourceConflict => "managed media source does not match its manifest",
            Self::ProviderRejected { .. } => "managed media provider rejected the transform",
            Self::OutputTooLarge => "managed media output exceeded its bound",
            Self::OutputIncompatible => "managed media output is incompatible",
            Self::StorageConflict => "managed media immutable object conflicts",
            Self::StorageFailure => "managed media storage operation failed",
        })
    }
}

impl std::error::Error for CloudflareMediaAdapterError {}

pub(crate) struct CloudflareMediaBindingAdapter;

pub(crate) fn binding_available(env: &Env) -> bool {
    Reflect::get(env, &JsValue::from_str(MEDIA_BINDING))
        .is_ok_and(|binding| !binding.is_null() && !binding.is_undefined())
}

impl CloudflareMediaBindingAdapter {
    pub(crate) async fn execute_to_staging(
        env: &Env,
        request: &CloudflareMediaRequest,
    ) -> Result<CloudflareStagedOutput, CloudflareMediaAdapterError> {
        request.validate()?;
        let bucket = env
            .bucket(RECORDINGS_BINDING)
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        if let Some(existing) = bucket
            .head(&request.staging_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
        {
            return staged_from_existing(request, &existing);
        }
        let source = bucket
            .get(&request.source_key)
            .execute()
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
            .ok_or(CloudflareMediaAdapterError::SourceMissing)?;
        let expected_source_checksum = parse_sha256(&request.source_sha256)
            .ok_or(CloudflareMediaAdapterError::InvalidRequest)?;
        let source_metadata = source.http_metadata();
        if source.size() != request.source_bytes
            || source.checksum().sha256.as_deref() != Some(expected_source_checksum.as_slice())
            || source_metadata.content_type.as_deref() != Some(request.source_content_type.as_str())
            || source_metadata.content_encoding.is_some()
        {
            return Err(CloudflareMediaAdapterError::SourceConflict);
        }
        let source_body = source
            .body()
            .ok_or(CloudflareMediaAdapterError::SourceMissing)?
            .stream()
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        let raw_input = wasm_streams::ReadableStream::from_stream(
            source_body
                .map_ok(|chunk| {
                    let array = Uint8Array::new_with_length(chunk.len() as u32);
                    array.copy_from(&chunk);
                    JsValue::from(array)
                })
                .map_err(|_| JsValue::from_str("managed_media_source_stream_failed")),
        )
        .into_raw()
        .unchecked_into::<ReadableStream>();

        let binding = Reflect::get(env, &JsValue::from_str(MEDIA_BINDING))
            .map_err(|_| CloudflareMediaAdapterError::BindingUnavailable)?;
        if binding.is_null() || binding.is_undefined() {
            return Err(CloudflareMediaAdapterError::BindingUnavailable);
        }
        let result = build_transform(&binding, raw_input, request)?;
        let provider_content_type = call_async(&result, "contentType", &[])
            .await?
            .as_string()
            .ok_or(CloudflareMediaAdapterError::OutputIncompatible)?;
        if provider_content_type != request.format.content_type() {
            return Err(CloudflareMediaAdapterError::OutputIncompatible);
        }
        let raw_output = call_async(&result, "media", &[])
            .await?
            .dyn_into::<ReadableStream>()
            .map_err(|_| CloudflareMediaAdapterError::OutputIncompatible)?;
        let bytes = collect_bounded(raw_output, request.max_output_bytes).await?;
        if !output_signature_matches(request.format, &bytes) {
            return Err(CloudflareMediaAdapterError::OutputIncompatible);
        }
        let checksum_bytes: [u8; 32] = Sha256::digest(&bytes).into();
        let checksum_sha256 = hex_sha256(&checksum_bytes);
        let metadata = HashMap::from([
            ("source-sha256".into(), request.source_sha256.clone()),
            ("profile-sha256".into(), request.profile_sha256.clone()),
            ("executor".into(), "cloudflare-media-binding-v1".into()),
        ]);
        let created = bucket
            .put(&request.staging_key, bytes)
            .http_metadata(private_http_metadata(&provider_content_type))
            .custom_metadata(metadata)
            .sha256(checksum_bytes.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        let object = match created {
            Some(object) => object,
            None => bucket
                .head(&request.staging_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .ok_or(CloudflareMediaAdapterError::StorageConflict)?,
        };
        let staged = CloudflareStagedOutput {
            staging_key: request.staging_key.clone(),
            final_key: request.final_key.clone(),
            bytes: object.size(),
            checksum_sha256,
            content_type: provider_content_type,
            source_sha256: request.source_sha256.clone(),
            profile_sha256: request.profile_sha256.clone(),
            estimated_provider_operations: request.estimated_provider_operations(),
            provider_output_seconds: request.duration_ms.unwrap_or(0).div_ceil(1_000),
        };
        verify_object(&object, &staged, true)?;
        Ok(staged)
    }

    pub(crate) async fn publish_staged(
        env: &Env,
        staged: &CloudflareStagedOutput,
    ) -> Result<bool, CloudflareMediaAdapterError> {
        validate_staged(staged)?;
        let bucket = env
            .bucket(RECORDINGS_BINDING)
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        if let Some(final_object) = bucket
            .head(&staged.final_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
        {
            verify_object(&final_object, staged, false)?;
            bucket
                .delete(&staged.staging_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
            return Ok(true);
        }
        let staging_object = bucket
            .get(&staged.staging_key)
            .execute()
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
            .ok_or(CloudflareMediaAdapterError::StorageConflict)?;
        verify_object(&staging_object, staged, true)?;
        let checksum = parse_sha256(&staged.checksum_sha256)
            .ok_or(CloudflareMediaAdapterError::StorageConflict)?;
        let stream = staging_object
            .body()
            .ok_or(CloudflareMediaAdapterError::StorageConflict)?
            .stream()
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        let fixed = FixedLengthStream::wrap(stream, staged.bytes);
        let created = bucket
            .put(&staged.final_key, fixed)
            .http_metadata(private_http_metadata(&staged.content_type))
            .custom_metadata(HashMap::from([
                ("source-sha256".into(), staged.source_sha256.clone()),
                ("profile-sha256".into(), staged.profile_sha256.clone()),
                ("executor".into(), "cloudflare-media-binding-v1".into()),
            ]))
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        let reused = created.is_none();
        let final_object = match created {
            Some(object) => object,
            None => bucket
                .head(&staged.final_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .ok_or(CloudflareMediaAdapterError::StorageConflict)?,
        };
        verify_object(&final_object, staged, false)?;
        bucket
            .delete(&staged.staging_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        Ok(reused)
    }

    pub(crate) async fn cancel_and_confirm_absent(
        env: &Env,
        staged: &CloudflareStagedOutput,
    ) -> Result<bool, CloudflareMediaAdapterError> {
        validate_staged(staged)?;
        let bucket = env
            .bucket(RECORDINGS_BINDING)
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        bucket
            .delete(&staged.staging_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        if let Some(final_object) = bucket
            .head(&staged.final_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
        {
            // Only delete the exact artifact owned by this fenced execution.
            // A collision or unrelated immutable object is never removed.
            verify_object(&final_object, staged, false)?;
            bucket
                .delete(&staged.final_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        }
        for _ in 0..2 {
            let staging_absent = bucket
                .head(&staged.staging_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .is_none();
            let final_absent = bucket
                .head(&staged.final_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .is_none();
            if !staging_absent || !final_absent {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) async fn cancel_planned_and_confirm_absent(
        env: &Env,
        plan: &CloudflareCancellationPlan,
    ) -> Result<bool, CloudflareMediaAdapterError> {
        plan.validate()?;
        let bucket = env
            .bucket(RECORDINGS_BINDING)
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;

        // A staging key is attempt-specific and is never a published object.
        bucket
            .delete(&plan.staging_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;

        if let Some(final_object) = bucket
            .head(&plan.final_key)
            .await
            .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
        {
            let metadata = final_object.http_metadata();
            let custom = final_object
                .custom_metadata()
                .map_err(|_| CloudflareMediaAdapterError::StorageConflict)?;
            let checksum_present = final_object
                .checksum()
                .sha256
                .as_deref()
                .is_some_and(|checksum| checksum.len() == 32);
            if final_object.key() != plan.final_key
                || final_object.size() == 0
                || !checksum_present
                || metadata.content_type.as_deref() != Some(plan.content_type.as_str())
                || metadata.content_encoding.is_some()
                || metadata.cache_control.as_deref() != Some("private, no-store")
                || custom.get("source-sha256") != Some(&plan.source_sha256)
                || custom.get("profile-sha256") != Some(&plan.profile_sha256)
                || custom.get("executor").map(String::as_str) != Some("cloudflare-media-binding-v1")
            {
                return Err(CloudflareMediaAdapterError::StorageConflict);
            }
            bucket
                .delete(&plan.final_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?;
        }

        for _ in 0..2 {
            let staging_absent = bucket
                .head(&plan.staging_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .is_none();
            let final_absent = bucket
                .head(&plan.final_key)
                .await
                .map_err(|_| CloudflareMediaAdapterError::StorageFailure)?
                .is_none();
            if !staging_absent || !final_absent {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn build_transform(
    binding: &JsValue,
    input: ReadableStream,
    request: &CloudflareMediaRequest,
) -> Result<JsValue, CloudflareMediaAdapterError> {
    let mut builder = call_sync(binding, "input", &[input.into()])?;
    if let (Some(width), Some(height)) = (request.width, request.height) {
        let options = Object::new();
        set(&options, "width", JsValue::from_f64(f64::from(width)))?;
        set(&options, "height", JsValue::from_f64(f64::from(height)))?;
        set(&options, "fit", JsValue::from_str(request.fit))?;
        builder = call_sync(&builder, "transform", &[options.into()])?;
    }
    let options = Object::new();
    set(
        &options,
        "mode",
        JsValue::from_str(request.mode.as_binding_value()),
    )?;
    set(
        &options,
        "time",
        JsValue::from_str(&seconds(request.start_ms)),
    )?;
    if let Some(duration) = request.duration_ms {
        set(&options, "duration", JsValue::from_str(&seconds(duration)))?;
    }
    if let Some(image_count) = request.image_count {
        set(
            &options,
            "imageCount",
            JsValue::from_f64(f64::from(image_count)),
        )?;
    }
    if let Some(format) = request.format.binding_value() {
        set(&options, "format", JsValue::from_str(format))?;
    }
    if request.mode == CloudflareMediaMode::Video {
        set(&options, "audio", JsValue::from_bool(request.include_audio))?;
    }
    call_sync(&builder, "output", &[options.into()])
}

async fn collect_bounded(
    stream: ReadableStream,
    maximum: u64,
) -> Result<Vec<u8>, CloudflareMediaAdapterError> {
    let mut output = Vec::new();
    let mut stream = ByteStream::from(stream);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| CloudflareMediaAdapterError::OutputIncompatible)?;
        let next = output
            .len()
            .checked_add(chunk.len())
            .ok_or(CloudflareMediaAdapterError::OutputTooLarge)?;
        if next as u64 > maximum || next as u64 > MAX_BUFFERED_OUTPUT_BYTES {
            return Err(CloudflareMediaAdapterError::OutputTooLarge);
        }
        output.extend_from_slice(&chunk);
    }
    if output.is_empty() {
        return Err(CloudflareMediaAdapterError::OutputIncompatible);
    }
    Ok(output)
}

fn staged_from_existing(
    request: &CloudflareMediaRequest,
    object: &worker::Object,
) -> Result<CloudflareStagedOutput, CloudflareMediaAdapterError> {
    let checksum = object
        .checksum()
        .sha256
        .as_deref()
        .map(hex_sha256)
        .ok_or(CloudflareMediaAdapterError::StorageConflict)?;
    let staged = CloudflareStagedOutput {
        staging_key: request.staging_key.clone(),
        final_key: request.final_key.clone(),
        bytes: object.size(),
        checksum_sha256: checksum,
        content_type: request.format.content_type().into(),
        source_sha256: request.source_sha256.clone(),
        profile_sha256: request.profile_sha256.clone(),
        estimated_provider_operations: 0,
        provider_output_seconds: 0,
    };
    verify_object(object, &staged, true)?;
    let metadata = object
        .custom_metadata()
        .map_err(|_| CloudflareMediaAdapterError::StorageConflict)?;
    if metadata.get("source-sha256") != Some(&request.source_sha256)
        || metadata.get("profile-sha256") != Some(&request.profile_sha256)
        || metadata.get("executor").map(String::as_str) != Some("cloudflare-media-binding-v1")
    {
        return Err(CloudflareMediaAdapterError::StorageConflict);
    }
    Ok(staged)
}

fn validate_staged(staged: &CloudflareStagedOutput) -> Result<(), CloudflareMediaAdapterError> {
    if !safe_private_key(&staged.staging_key)
        || !safe_private_key(&staged.final_key)
        || !valid_staging_key(&staged.staging_key, &staged.final_key)
        || staged.final_key.ends_with(".partial")
        || staged.bytes == 0
        || staged.bytes > MAX_BUFFERED_OUTPUT_BYTES
        || !valid_sha256(&staged.checksum_sha256)
        || !valid_sha256(&staged.source_sha256)
        || !valid_sha256(&staged.profile_sha256)
        || !matches!(
            staged.content_type.as_str(),
            "video/mp4" | "image/jpeg" | "image/png" | "audio/mp4"
        )
    {
        return Err(CloudflareMediaAdapterError::StorageConflict);
    }
    Ok(())
}

fn verify_object(
    object: &worker::Object,
    staged: &CloudflareStagedOutput,
    staging: bool,
) -> Result<(), CloudflareMediaAdapterError> {
    validate_staged(staged)?;
    let checksum = parse_sha256(&staged.checksum_sha256)
        .ok_or(CloudflareMediaAdapterError::StorageConflict)?;
    let metadata = object.http_metadata();
    let expected_key = if staging {
        &staged.staging_key
    } else {
        &staged.final_key
    };
    if object.key() != *expected_key
        || object.size() != staged.bytes
        || object.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(staged.content_type.as_str())
        || metadata.content_encoding.is_some()
        || metadata.cache_control.as_deref() != Some("private, no-store")
    {
        return Err(CloudflareMediaAdapterError::StorageConflict);
    }
    Ok(())
}

fn private_http_metadata(content_type: &str) -> HttpMetadata {
    HttpMetadata {
        content_type: Some(content_type.into()),
        content_disposition: Some("inline".into()),
        cache_control: Some("private, no-store".into()),
        ..HttpMetadata::default()
    }
}

fn call_sync(
    target: &JsValue,
    name: &str,
    arguments: &[JsValue],
) -> Result<JsValue, CloudflareMediaAdapterError> {
    let function = Reflect::get(target, &JsValue::from_str(name))
        .map_err(provider_error)?
        .dyn_into::<Function>()
        .map_err(provider_error)?;
    let args = Array::new();
    for argument in arguments {
        args.push(argument);
    }
    function.apply(target, &args).map_err(provider_error)
}

async fn call_async(
    target: &JsValue,
    name: &str,
    arguments: &[JsValue],
) -> Result<JsValue, CloudflareMediaAdapterError> {
    let value = call_sync(target, name, arguments)?;
    let promise = value.dyn_into::<Promise>().map_err(provider_error)?;
    JsFuture::from(promise).await.map_err(provider_error)
}

fn provider_error(value: JsValue) -> CloudflareMediaAdapterError {
    let code = Reflect::get(&value, &JsValue::from_str("code"))
        .ok()
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= f64::from(u32::MAX))
        .map(|value| value as u32);
    CloudflareMediaAdapterError::ProviderRejected { code }
}

fn set(object: &Object, name: &str, value: JsValue) -> Result<(), CloudflareMediaAdapterError> {
    Reflect::set(object, &JsValue::from_str(name), &value)
        .map(|_| ())
        .map_err(provider_error)
}

fn seconds(milliseconds: u64) -> String {
    format!("{}s", milliseconds / 1_000)
}

fn output_signature_matches(format: CloudflareMediaFormat, bytes: &[u8]) -> bool {
    match format {
        CloudflareMediaFormat::Jpeg => {
            bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9])
        }
        CloudflareMediaFormat::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        CloudflareMediaFormat::Mp4H264Aac => {
            contains_marker(&bytes[..bytes.len().min(64)], b"ftyp")
                && contains_marker(bytes, b"moov")
                && contains_marker(bytes, b"avc1")
        }
        CloudflareMediaFormat::M4aAac => {
            contains_marker(&bytes[..bytes.len().min(64)], b"ftyp")
                && contains_marker(bytes, b"moov")
                && contains_marker(bytes, b"mp4a")
        }
    }
}

fn contains_marker(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn safe_private_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1_024
        && !value.contains("://")
        && !value.contains(['?', '#', '\\', '%'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
}

fn valid_staging_key(staging: &str, final_key: &str) -> bool {
    staging
        .strip_prefix(final_key)
        .and_then(|suffix| suffix.strip_prefix(".attempt-"))
        .and_then(|suffix| suffix.strip_suffix(".partial"))
        .is_some_and(|attempt| {
            !attempt.is_empty()
                && attempt.len() <= 5
                && attempt.bytes().all(|byte| byte.is_ascii_digit())
                && attempt.parse::<u16>().is_ok_and(|value| value > 0)
        })
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == value)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    if !valid_sha256(value) {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        output[index] = (high << 4) | low;
    }
    Some(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn hex_sha256(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "018f0b5f-1f52-7c2d-8c50-2c2dc5f2a101";
    const VIDEO: &str = "018f0b5f-1f52-7c2d-8c50-2c2dc5f2a102";

    fn request(mode: CloudflareMediaMode) -> CloudflareMediaRequest {
        let final_key = format!(
            "tenants/{TENANT}/videos/{VIDEO}/derivatives/optimized_clip_v1/{}",
            "c".repeat(64)
        );
        CloudflareMediaRequest {
            tenant_id: TENANT.into(),
            video_id: VIDEO.into(),
            source_key: format!("tenants/{TENANT}/videos/{VIDEO}/source/v1/input.mp4"),
            source_bytes: 1_000_000,
            source_sha256: "a".repeat(64),
            source_content_type: "video/mp4".into(),
            source_duration_ms: 120_000,
            profile_sha256: "b".repeat(64),
            staging_key: format!("{final_key}.attempt-1.partial"),
            final_key,
            mode,
            format: CloudflareMediaFormat::Mp4H264Aac,
            start_ms: 0,
            duration_ms: Some(5_000),
            width: Some(640),
            height: Some(360),
            fit: "contain",
            image_count: None,
            include_audio: true,
            max_output_bytes: 8_000_000,
        }
    }

    #[test]
    fn exact_binding_contract_is_bounded_and_private() {
        let request = request(CloudflareMediaMode::Video);
        request.validate().expect("valid request");
        assert_eq!(request.estimated_provider_operations(), 5);
        let debug = format!("{request:?}");
        assert!(!debug.contains(TENANT));
        assert!(!debug.contains(VIDEO));
        assert!(!debug.contains(&request.source_key));
        assert!(!debug.contains(&request.source_sha256));
    }

    #[test]
    fn exact_and_just_over_boundaries_fail_closed() {
        let mut value = request(CloudflareMediaMode::Video);
        value.source_bytes = MAX_INPUT_BYTES_EXCLUSIVE - 1;
        value.validate().expect("exact byte boundary");
        value.source_bytes += 1;
        assert_eq!(
            value.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );

        let mut value = request(CloudflareMediaMode::Video);
        value.duration_ms = Some(MAX_OUTPUT_DURATION_MS);
        value.validate().expect("exact duration boundary");
        value.duration_ms = Some(MAX_OUTPUT_DURATION_MS + 1);
        assert_eq!(
            value.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );

        let mut value = request(CloudflareMediaMode::Video);
        value.width = Some(MAX_DIMENSION);
        value.height = Some(MIN_DIMENSION);
        value.validate().expect("exact dimension boundaries");
        value.width = Some(MAX_DIMENSION + 1);
        assert_eq!(
            value.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );

        let mut value = request(CloudflareMediaMode::Video);
        value.max_output_bytes = MAX_BUFFERED_OUTPUT_BYTES + 1;
        assert_eq!(
            value.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );
    }

    #[test]
    fn mode_shapes_and_time_precision_are_exact() {
        assert_eq!(
            CloudflareMediaMode::Video.implementation_id(),
            "cloudflare_video_bounded_v1"
        );
        assert_eq!(
            CloudflareMediaMode::Frame.implementation_id(),
            "cloudflare_frame_bounded_v1"
        );
        assert_eq!(
            CloudflareMediaMode::Spritesheet.implementation_id(),
            "cloudflare_spritesheet_bounded_v1"
        );
        assert_eq!(
            CloudflareMediaMode::Audio.implementation_id(),
            "cloudflare_audio_bounded_v1"
        );
        let mut frame = request(CloudflareMediaMode::Frame);
        frame.format = CloudflareMediaFormat::Jpeg;
        frame.duration_ms = None;
        frame.include_audio = false;
        frame.validate().expect("frame");

        let mut spritesheet = request(CloudflareMediaMode::Spritesheet);
        spritesheet.format = CloudflareMediaFormat::Jpeg;
        spritesheet.image_count = Some(MAX_IMAGE_COUNT);
        spritesheet.include_audio = false;
        spritesheet.validate().expect("spritesheet");
        spritesheet.image_count = Some(MAX_IMAGE_COUNT + 1);
        assert_eq!(
            spritesheet.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );

        let mut audio = request(CloudflareMediaMode::Audio);
        audio.format = CloudflareMediaFormat::M4aAac;
        audio.width = None;
        audio.height = None;
        audio.include_audio = false;
        audio.validate().expect("audio");

        let mut fractional = request(CloudflareMediaMode::Video);
        fractional.start_ms = 1;
        assert_eq!(
            fractional.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );
    }

    #[test]
    fn signatures_and_scoped_keys_reject_ambiguity() {
        assert!(output_signature_matches(
            CloudflareMediaFormat::Jpeg,
            &[0xff, 0xd8, 0, 0xff, 0xd9]
        ));
        assert!(output_signature_matches(
            CloudflareMediaFormat::Png,
            b"\x89PNG\r\n\x1a\nbody"
        ));
        assert!(output_signature_matches(
            CloudflareMediaFormat::Mp4H264Aac,
            b"0000ftyp0000moov0000avc1"
        ));
        assert!(!output_signature_matches(
            CloudflareMediaFormat::M4aAac,
            b"0000ftyp0000moov0000avc1"
        ));

        let mut invalid = request(CloudflareMediaMode::Video);
        invalid.source_key = "https://private.example/source.mp4".into();
        assert_eq!(
            invalid.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );
        let mut invalid = request(CloudflareMediaMode::Video);
        invalid.staging_key = format!("{}.partial", invalid.source_key);
        assert_eq!(
            invalid.validate(),
            Err(CloudflareMediaAdapterError::InvalidRequest)
        );
    }

    #[test]
    fn cancellation_plan_binds_attempt_and_immutable_identity() {
        let request = request(CloudflareMediaMode::Video);
        let plan = CloudflareCancellationPlan {
            staging_key: request.staging_key.clone(),
            final_key: request.final_key.clone(),
            source_sha256: request.source_sha256.clone(),
            profile_sha256: request.profile_sha256.clone(),
            content_type: request.format.content_type().into(),
        };
        plan.validate().expect("valid recovery plan");

        let mut swapped = plan.clone();
        swapped.staging_key = format!("{}.attempt-2.partial", request.source_key);
        assert_eq!(
            swapped.validate(),
            Err(CloudflareMediaAdapterError::StorageConflict)
        );
        let mut invalid = plan;
        invalid.profile_sha256 = "A".repeat(64);
        assert_eq!(
            invalid.validate(),
            Err(CloudflareMediaAdapterError::StorageConflict)
        );
    }
}
