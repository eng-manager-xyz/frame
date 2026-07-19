//! Safe, bounded macOS system-audio capture primitives.
//!
//! Platform-independent format, identity, and buffer-shape validation live in
//! this module so their hard bounds remain testable on every workspace host.

#![forbid(unsafe_code)]

use std::fmt;

use frame_media::{AudioFormat, AudioSampleFormat, AvDeviceId, PermissionState};
use ring::hmac;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod platform;

#[cfg(target_os = "macos")]
pub use platform::MacOsSystemAudioSource;

pub const SYSTEM_AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub const SYSTEM_AUDIO_CHANNELS: u8 = 2;
pub const F32_BYTES_PER_SAMPLE: usize = size_of::<f32>();
/// Reject a native callback larger than 100 ms of exact stereo F32 audio.
pub const MAX_AUDIO_CHUNK_FRAMES: u32 = 4_800;
pub const MAX_AUDIO_CHUNK_BYTES: usize =
    MAX_AUDIO_CHUNK_FRAMES as usize * SYSTEM_AUDIO_CHANNELS as usize * F32_BYTES_PER_SAMPLE;
/// At most 1.6 seconds / 600 KiB at the per-callback maximum.
pub const AUDIO_CALLBACK_QUEUE_CAPACITY: usize = 16;
pub const MAX_CALLBACK_QUEUED_BYTES: usize = AUDIO_CALLBACK_QUEUE_CAPACITY * MAX_AUDIO_CHUNK_BYTES;
pub const MAX_CALLBACK_QUEUED_DURATION_NS: u64 =
    AUDIO_CALLBACK_QUEUE_CAPACITY as u64 * MAX_AUDIO_CHUNK_FRAMES as u64 * 1_000_000_000
        / SYSTEM_AUDIO_SAMPLE_RATE_HZ as u64;
const _: () = assert!(MAX_CALLBACK_QUEUED_DURATION_NS <= 2_000_000_000);

pub const SYSTEM_AUDIO_FORMAT: AudioFormat = AudioFormat {
    sample_rate: SYSTEM_AUDIO_SAMPLE_RATE_HZ,
    channels: SYSTEM_AUDIO_CHANNELS,
    sample_format: AudioSampleFormat::Float32,
};

const SYSTEM_AUDIO_DEVICE_ID_DOMAIN: &[u8] = b"frame/macos-system-audio-device/v1\0";

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MacOsSystemAudioError {
    #[error("the installation secret must contain entropy")]
    InvalidInstallationSecret,
    #[error("the stable system-audio device ID could not be derived")]
    DeviceIdDerivationFailed,
    #[error("screen-recording permission is not granted")]
    PermissionDenied,
    #[error("a system-audio stream is already running")]
    AlreadyRunning,
    #[error("no system-audio stream is running")]
    NotRunning,
    #[error("ScreenCaptureKit shareable content is unavailable")]
    ShareableContentUnavailable,
    #[error("ScreenCaptureKit exposed no display for the system-audio filter")]
    NoDisplayAvailable,
    #[error("another bounded ScreenCaptureKit operation owns native-call capacity")]
    NativeOperationCapacityUnavailable,
    #[error("the bounded ScreenCaptureKit operation worker could not be started")]
    NativeOperationWorkerUnavailable,
    #[error("the ScreenCaptureKit operation did not complete before its deadline")]
    NativeOperationTimedOut,
    #[error("ScreenCaptureKit rejected the system-audio output handler")]
    OutputHandlerRegistrationFailed,
    #[error("ScreenCaptureKit could not start system-audio capture")]
    CaptureStartFailed,
    #[error("system-audio start failed and native teardown was not confirmed")]
    CaptureStartTeardownUnconfirmed,
    #[error("ScreenCaptureKit reported an unexpected system-audio stop")]
    UnexpectedStreamStop,
    #[error("ScreenCaptureKit could not stop system-audio capture")]
    CaptureStopFailed,
    #[error("system-audio teardown remains unconfirmed; this source cannot be reused")]
    CaptureTeardownUnconfirmed,
    #[error("ScreenCaptureKit output-handler release could not be confirmed")]
    OutputHandlerReleaseUnconfirmed,
    #[error("the serial audio callback queue did not reach its fence before the deadline")]
    CallbackQueueFenceTimedOut,
    #[error("the ScreenCaptureKit delegate did not quiesce before the deadline")]
    DelegateQuiescenceUnconfirmed,
    #[error("the bounded audio callback queue disconnected unexpectedly")]
    CallbackQueueDisconnected,
    #[error("the native audio sample buffer is invalid or not ready")]
    InvalidSampleBuffer,
    #[error("the native sample is not exact 48 kHz stereo F32LE PCM")]
    UnexpectedAudioFormat,
    #[error("the native audio sample has no readable audio buffers")]
    MissingAudioBuffer,
    #[error("the native audio buffers are neither interleaved stereo nor two planar channels")]
    InvalidAudioBufferLayout,
    #[error("the native audio chunk exceeds the 100 ms byte/frame ceiling")]
    AudioChunkTooLarge,
    #[error("the native audio sample contains a non-finite F32 value")]
    NonFiniteAudioSample,
    #[error("the native audio timestamp is invalid or overflows nanoseconds")]
    InvalidTimestamp,
    #[error("the system-audio callback sequence exhausted its range")]
    SequenceExhausted,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MacOsSystemAudioStopError {
    #[error("ScreenCaptureKit native stop was not confirmed: {0}")]
    NativeStopUnconfirmed(#[source] MacOsSystemAudioError),
    #[error("ScreenCaptureKit stopped but callback quiescence was not confirmed: {0}")]
    CallbackQuiescenceUnconfirmed(#[source] MacOsSystemAudioError),
    #[error("system-audio capture failed after confirmed teardown: {0}")]
    CaptureFailedAfterTeardown(#[source] MacOsSystemAudioError),
}

impl MacOsSystemAudioStopError {
    #[must_use]
    pub const fn native_stop_confirmed(self) -> bool {
        !matches!(self, Self::NativeStopUnconfirmed(_))
    }

    #[must_use]
    pub const fn capture_teardown_confirmed(self) -> bool {
        matches!(self, Self::CaptureFailedAfterTeardown(_))
    }

    #[must_use]
    pub const fn capture_error(self) -> MacOsSystemAudioError {
        match self {
            Self::NativeStopUnconfirmed(error)
            | Self::CallbackQuiescenceUnconfirmed(error)
            | Self::CaptureFailedAfterTeardown(error) => error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacOsSystemAudioDiagnostics {
    pub dropped_callback_chunks: u64,
    pub callback_chunks_after_stop: u64,
    pub invalid_callback_chunks: u64,
    pub unexpected_native_stops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacOsSystemAudioDevice {
    id: AvDeviceId,
    permission: PermissionState,
}

impl MacOsSystemAudioDevice {
    #[must_use]
    pub const fn id(self) -> AvDeviceId {
        self.id
    }

    #[must_use]
    pub const fn format(self) -> AudioFormat {
        SYSTEM_AUDIO_FORMAT
    }

    #[must_use]
    pub const fn permission(self) -> PermissionState {
        self.permission
    }
}

pub struct MacOsSystemAudioChunk {
    sequence: u64,
    source_pts_ns: u64,
    duration_ns: u64,
    discontinuity: bool,
    samples_f32le: Vec<u8>,
}

impl MacOsSystemAudioChunk {
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn source_pts_ns(&self) -> u64 {
        self.source_pts_ns
    }

    #[must_use]
    pub const fn duration_ns(&self) -> u64 {
        self.duration_ns
    }

    #[must_use]
    pub const fn discontinuity(&self) -> bool {
        self.discontinuity
    }

    #[must_use]
    pub const fn format(&self) -> AudioFormat {
        SYSTEM_AUDIO_FORMAT
    }

    #[must_use]
    pub fn samples_f32le(&self) -> &[u8] {
        &self.samples_f32le
    }

    #[must_use]
    pub fn into_samples_f32le(self) -> Vec<u8> {
        self.samples_f32le
    }
}

impl fmt::Debug for MacOsSystemAudioChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacOsSystemAudioChunk")
            .field("sequence", &self.sequence)
            .field("source_pts_ns", &"<redacted>")
            .field("duration_ns", &self.duration_ns)
            .field("discontinuity", &self.discontinuity)
            .field("retained_bytes", &self.samples_f32le.len())
            .finish()
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NativeAudioDescription {
    pub sample_rate_hz: f64,
    pub channels: u32,
    pub bits_per_channel: u32,
    pub pcm: bool,
    pub float: bool,
    pub big_endian: bool,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy)]
pub(crate) struct AudioPlane<'a> {
    pub channels: u32,
    pub bytes: &'a [u8],
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn validate_audio_description(
    description: NativeAudioDescription,
) -> Result<(), MacOsSystemAudioError> {
    if description.sample_rate_hz != f64::from(SYSTEM_AUDIO_SAMPLE_RATE_HZ)
        || description.channels != u32::from(SYSTEM_AUDIO_CHANNELS)
        || description.bits_per_channel != 32
        || !description.pcm
        || !description.float
        || description.big_endian
    {
        return Err(MacOsSystemAudioError::UnexpectedAudioFormat);
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn extract_stereo_f32le(
    frames: u32,
    planes: &[AudioPlane<'_>],
) -> Result<Vec<u8>, MacOsSystemAudioError> {
    if frames == 0 {
        return Err(MacOsSystemAudioError::InvalidAudioBufferLayout);
    }
    if frames > MAX_AUDIO_CHUNK_FRAMES {
        return Err(MacOsSystemAudioError::AudioChunkTooLarge);
    }
    let frames = usize::try_from(frames).map_err(|_| MacOsSystemAudioError::AudioChunkTooLarge)?;
    let channel_bytes = frames
        .checked_mul(F32_BYTES_PER_SAMPLE)
        .ok_or(MacOsSystemAudioError::AudioChunkTooLarge)?;
    let interleaved_bytes = channel_bytes
        .checked_mul(usize::from(SYSTEM_AUDIO_CHANNELS))
        .ok_or(MacOsSystemAudioError::AudioChunkTooLarge)?;
    if interleaved_bytes > MAX_AUDIO_CHUNK_BYTES {
        return Err(MacOsSystemAudioError::AudioChunkTooLarge);
    }

    match planes {
        [interleaved]
            if interleaved.channels == u32::from(SYSTEM_AUDIO_CHANNELS)
                && interleaved.bytes.len() == interleaved_bytes =>
        {
            validate_finite_samples(interleaved.bytes)?;
            Ok(interleaved.bytes.to_vec())
        }
        [left, right]
            if left.channels == 1
                && right.channels == 1
                && left.bytes.len() == channel_bytes
                && right.bytes.len() == channel_bytes =>
        {
            validate_finite_samples(left.bytes)?;
            validate_finite_samples(right.bytes)?;
            let mut interleaved = Vec::with_capacity(interleaved_bytes);
            for frame in 0..frames {
                let offset = frame * F32_BYTES_PER_SAMPLE;
                interleaved.extend_from_slice(&left.bytes[offset..offset + F32_BYTES_PER_SAMPLE]);
                interleaved.extend_from_slice(&right.bytes[offset..offset + F32_BYTES_PER_SAMPLE]);
            }
            Ok(interleaved)
        }
        _ => Err(MacOsSystemAudioError::InvalidAudioBufferLayout),
    }
}

#[cfg(any(target_os = "macos", test))]
fn validate_finite_samples(bytes: &[u8]) -> Result<(), MacOsSystemAudioError> {
    let (samples, remainder) = bytes.as_chunks::<4>();
    if !remainder.is_empty()
        || samples
            .iter()
            .any(|sample| !f32::from_le_bytes(*sample).is_finite())
    {
        return Err(MacOsSystemAudioError::NonFiniteAudioSample);
    }
    Ok(())
}

pub fn derive_system_audio_device_id(
    installation_secret: &[u8; 32],
) -> Result<AvDeviceId, MacOsSystemAudioError> {
    if installation_secret.iter().all(|byte| *byte == 0) {
        return Err(MacOsSystemAudioError::InvalidInstallationSecret);
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, installation_secret);
    let tag = hmac::sign(&key, SYSTEM_AUDIO_DEVICE_ID_DOMAIN);
    let mut opaque = [0_u8; 16];
    opaque.copy_from_slice(&tag.as_ref()[..16]);
    AvDeviceId::from_opaque(opaque).map_err(|_| MacOsSystemAudioError::DeviceIdDerivationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32le(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn exact_native_audio_description_is_required() {
        let exact = NativeAudioDescription {
            sample_rate_hz: 48_000.0,
            channels: 2,
            bits_per_channel: 32,
            pcm: true,
            float: true,
            big_endian: false,
        };
        assert_eq!(validate_audio_description(exact), Ok(()));
        assert_eq!(
            validate_audio_description(NativeAudioDescription {
                sample_rate_hz: 44_100.0,
                ..exact
            }),
            Err(MacOsSystemAudioError::UnexpectedAudioFormat)
        );
        assert_eq!(
            validate_audio_description(NativeAudioDescription {
                big_endian: true,
                ..exact
            }),
            Err(MacOsSystemAudioError::UnexpectedAudioFormat)
        );
    }

    #[test]
    fn interleaved_and_planar_stereo_produce_the_same_f32le_bytes() {
        let expected = f32le(&[0.25, -0.5, 0.75, -1.0]);
        let interleaved = extract_stereo_f32le(
            2,
            &[AudioPlane {
                channels: 2,
                bytes: &expected,
            }],
        )
        .expect("interleaved stereo");
        let left = f32le(&[0.25, 0.75]);
        let right = f32le(&[-0.5, -1.0]);
        let planar = extract_stereo_f32le(
            2,
            &[
                AudioPlane {
                    channels: 1,
                    bytes: &left,
                },
                AudioPlane {
                    channels: 1,
                    bytes: &right,
                },
            ],
        )
        .expect("planar stereo");
        assert_eq!(interleaved, expected);
        assert_eq!(planar, expected);
    }

    #[test]
    fn extraction_rejects_layout_size_nonfinite_and_frame_overflow() {
        let short = f32le(&[0.0]);
        assert_eq!(
            extract_stereo_f32le(
                1,
                &[AudioPlane {
                    channels: 2,
                    bytes: &short,
                }]
            ),
            Err(MacOsSystemAudioError::InvalidAudioBufferLayout)
        );
        let nonfinite = f32le(&[f32::NAN, 0.0]);
        assert_eq!(
            extract_stereo_f32le(
                1,
                &[AudioPlane {
                    channels: 2,
                    bytes: &nonfinite,
                }]
            ),
            Err(MacOsSystemAudioError::NonFiniteAudioSample)
        );
        assert_eq!(
            extract_stereo_f32le(MAX_AUDIO_CHUNK_FRAMES + 1, &[]),
            Err(MacOsSystemAudioError::AudioChunkTooLarge)
        );
    }

    #[test]
    fn stable_device_id_is_secret_bound_and_redacted() {
        let first = derive_system_audio_device_id(&[1; 32]).expect("first ID");
        assert_eq!(
            first,
            derive_system_audio_device_id(&[1; 32]).expect("stable ID")
        );
        assert_ne!(
            first,
            derive_system_audio_device_id(&[2; 32]).expect("installation-bound ID")
        );
        assert_eq!(format!("{first:?}"), "AvDeviceId(<redacted>)");
        assert_eq!(
            derive_system_audio_device_id(&[0; 32]),
            Err(MacOsSystemAudioError::InvalidInstallationSecret)
        );
    }

    #[test]
    fn callback_prequeue_stays_inside_the_two_second_ingress_ceiling() {
        assert_eq!(MAX_CALLBACK_QUEUED_DURATION_NS, 1_600_000_000);
        assert_eq!(MAX_CALLBACK_QUEUED_BYTES, 614_400);
    }
}
