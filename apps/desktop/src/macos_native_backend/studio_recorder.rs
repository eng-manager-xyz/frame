//! Desktop ownership boundary for durable native Studio originals.
//!
//! The macOS worker remains the sole owner of native capture. This adapter
//! gives that worker one bounded route into the provider-neutral Studio
//! encoder and filesystem session without exposing raw media through IPC.

use std::path::Path;

use frame_media::{
    AudioSampleFormat, BoundedMediaQueue, CancellationToken, CaptureElementFamily,
    FilesystemStudioOriginalStore, FilesystemStudioRecordingSession, FrameRate, FrameTimestamp,
    IsolatedTrackBranch, NativeStudioInputBuffer, NativeStudioRecording,
    NativeStudioRecordingArtifact, NativeStudioRecordingError, PixelFormat, StudioAssetEncoding,
    StudioAssetId, StudioAudioRawCaps, StudioOperationId, StudioProjectId,
    StudioRecordingGraphSpec, StudioSourceName, StudioVideoRawCaps, TrackKind, VideoFrameSpec,
};

use crate::NativeDesktopBackendError;

const STUDIO_QUEUE_BUFFERS: u32 = 64;
const STUDIO_QUEUE_BYTES: u64 = 256 * 1024 * 1024;
const STUDIO_QUEUE_TIME_NS: u64 = 2_000_000_000;
const MAXIMUM_STUDIO_TRACK_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StudioRecordingIdentity {
    pub(super) project: [u8; 16],
    pub(super) clock: [u8; 16],
    pub(super) screen_asset: [u8; 16],
    pub(super) microphone_asset: [u8; 16],
    pub(super) system_audio_asset: [u8; 16],
    pub(super) camera_asset: [u8; 16],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct StudioOptionalTracks {
    pub(super) microphone: bool,
    pub(super) system_audio: bool,
    pub(super) camera: bool,
}

/// One active, isolated-track Studio encoder and its durable session.
pub(super) struct DesktopStudioRecording {
    inner: NativeStudioRecording,
}

impl std::fmt::Debug for DesktopStudioRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopStudioRecording")
            .field("inner", &self.inner)
            .finish()
    }
}

impl DesktopStudioRecording {
    pub(super) fn start(
        root: &Path,
        identity: StudioRecordingIdentity,
        screen: VideoFrameSpec,
        frame_rate: u16,
        optional: StudioOptionalTracks,
    ) -> Result<Self, NativeDesktopBackendError> {
        let graph = recording_graph(identity, screen, frame_rate, optional)?;
        let store = FilesystemStudioOriginalStore::new(root).map_err(map_studio_error)?;
        let session = FilesystemStudioRecordingSession::begin(
            &store,
            graph.clone(),
            MAXIMUM_STUDIO_TRACK_BYTES,
        )
        .map_err(map_studio_error)?;
        let inner = NativeStudioRecording::start(&graph, session).map_err(map_native_error)?;
        Ok(Self { inner })
    }

    pub(super) fn push_screen(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
        pixels: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        self.push(TrackKind::Screen, sequence, timestamp, pixels)
    }

    pub(super) fn push_system_audio(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
        samples: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        self.push(TrackKind::SystemAudio, sequence, timestamp, samples)
    }

    fn push(
        &mut self,
        track: TrackKind,
        sequence: u64,
        timestamp: FrameTimestamp,
        bytes: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        let input = NativeStudioInputBuffer::new(track, sequence, timestamp, bytes)
            .map_err(map_native_error)?;
        self.inner.push(input).map_err(map_native_error)?;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<NativeStudioRecordingArtifact, NativeDesktopBackendError> {
        self.inner
            .finish(&CancellationToken::new())
            .map_err(map_native_error)
    }

    pub(super) fn abort(self) -> Result<(), NativeDesktopBackendError> {
        self.inner.abort().map_err(map_native_error)
    }
}

fn recording_graph(
    identity: StudioRecordingIdentity,
    screen: VideoFrameSpec,
    frame_rate: u16,
    optional: StudioOptionalTracks,
) -> Result<StudioRecordingGraphSpec, NativeDesktopBackendError> {
    let project = StudioProjectId::from_csprng(identity.project).map_err(map_studio_error)?;
    let clock = StudioOperationId::from_csprng(identity.clock).map_err(map_studio_error)?;
    let mut branches = vec![video_branch(
        TrackKind::Screen,
        identity.screen_asset,
        "screen.webm",
        screen.width,
        screen.height,
        frame_rate,
    )?];
    if optional.microphone {
        branches.push(audio_branch(
            TrackKind::Microphone,
            identity.microphone_asset,
            "microphone.webm",
        )?);
    }
    if optional.system_audio {
        branches.push(audio_branch(
            TrackKind::SystemAudio,
            identity.system_audio_asset,
            "system-audio.webm",
        )?);
    }
    if optional.camera {
        // The combined native bridge negotiates the canonical camera format.
        branches.push(video_branch(
            TrackKind::Camera,
            identity.camera_asset,
            "camera.webm",
            1_280,
            720,
            30,
        )?);
    }
    StudioRecordingGraphSpec::new(project, clock, branches).map_err(map_studio_error)
}

fn video_branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
    width: u32,
    height: u32,
    frame_rate: u16,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let encoding = StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
        width,
        height,
        frame_rate: FrameRate {
            numerator: u32::from(frame_rate),
            denominator: 1,
        },
        pixel_format: PixelFormat::Bgra8,
    })
    .map_err(map_studio_error)?;
    branch(track, id, name, CaptureElementFamily::Vp8Encoder, encoding)
}

fn audio_branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let encoding = StudioAssetEncoding::recording_opus_webm(StudioAudioRawCaps {
        sample_rate: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::Float32,
    })
    .map_err(map_studio_error)?;
    branch(track, id, name, CaptureElementFamily::OpusEncoder, encoding)
}

fn branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
    encoder: CaptureElementFamily,
    encoding: StudioAssetEncoding,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let source = match track {
        TrackKind::Screen => CaptureElementFamily::NativeScreenBridge,
        TrackKind::Camera => CaptureElementFamily::NativeCameraBridge,
        TrackKind::Microphone => CaptureElementFamily::NativeMicrophoneBridge,
        TrackKind::SystemAudio => CaptureElementFamily::NativeSystemAudioBridge,
    };
    Ok(IsolatedTrackBranch {
        track,
        asset_id: StudioAssetId::from_csprng(id).map_err(map_studio_error)?,
        temporary_name: StudioSourceName::new(name).map_err(map_studio_error)?,
        source,
        encoder,
        muxer: CaptureElementFamily::WebMMux,
        encoding,
        queue: BoundedMediaQueue {
            max_buffers: STUDIO_QUEUE_BUFFERS,
            max_bytes: STUDIO_QUEUE_BYTES,
            max_time_ns: STUDIO_QUEUE_TIME_NS,
        },
    })
}

fn map_studio_error(error: frame_media::StudioError) -> NativeDesktopBackendError {
    match error {
        frame_media::StudioError::StorageIo | frame_media::StudioError::UnsafeStoragePath => {
            NativeDesktopBackendError::Filesystem
        }
        _ => NativeDesktopBackendError::Internal,
    }
}

fn map_native_error(error: NativeStudioRecordingError) -> NativeDesktopBackendError {
    match error {
        NativeStudioRecordingError::Studio(frame_media::StudioError::StorageIo)
        | NativeStudioRecordingError::Studio(frame_media::StudioError::UnsafeStoragePath) => {
            NativeDesktopBackendError::Filesystem
        }
        _ => NativeDesktopBackendError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use frame_media::{ColorSpace, FrameMemory};

    use super::*;

    const VIDEO_DURATION_NS: u64 = 33_333_333;
    const AUDIO_DURATION_NS: u64 = 21_333_333;

    fn identity() -> StudioRecordingIdentity {
        StudioRecordingIdentity {
            project: [1; 16],
            clock: [2; 16],
            screen_asset: [3; 16],
            microphone_asset: [4; 16],
            system_audio_asset: [5; 16],
            camera_asset: [6; 16],
        }
    }

    fn screen_spec() -> VideoFrameSpec {
        VideoFrameSpec {
            width: 160,
            height: 90,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: VIDEO_DURATION_NS,
            memory: FrameMemory::Cpu,
        }
    }

    fn audio(sequence: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1_024 * 2 * size_of::<f32>());
        for frame in 0..1_024 {
            let sample = ((frame as f32 / 1_024.0) * TAU).sin() * 0.25;
            bytes.extend_from_slice(&sample.to_le_bytes());
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        assert!(sequence > 0);
        bytes
    }

    #[test]
    fn production_adapter_seals_screen_and_system_audio_originals() {
        let directory = tempfile::tempdir().expect("Studio root");
        let mut recording = DesktopStudioRecording::start(
            directory.path(),
            identity(),
            screen_spec(),
            30,
            StudioOptionalTracks {
                system_audio: true,
                ..StudioOptionalTracks::default()
            },
        )
        .expect("Studio recording");
        for sequence in 1..=30 {
            recording
                .push_screen(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * VIDEO_DURATION_NS, VIDEO_DURATION_NS)
                        .expect("video timestamp"),
                    vec![42; 160 * 90 * 4],
                )
                .expect("screen original");
        }
        for sequence in 1..=47 {
            recording
                .push_system_audio(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * AUDIO_DURATION_NS, AUDIO_DURATION_NS)
                        .expect("audio timestamp"),
                    audio(sequence),
                )
                .expect("system-audio original");
        }
        let artifact = recording.finish().expect("sealed Studio originals");
        assert_eq!(artifact.assets.len(), 2);
        assert_eq!(artifact.tracks.len(), 2);
        assert!(
            artifact
                .tracks
                .iter()
                .all(|track| track.submitted_buffers > 0 && track.encoded_bytes > 1_024)
        );
    }

    #[test]
    fn production_adapter_keeps_optional_tracks_optional() {
        let directory = tempfile::tempdir().expect("Studio root");
        let mut recording = DesktopStudioRecording::start(
            directory.path(),
            identity(),
            screen_spec(),
            30,
            StudioOptionalTracks::default(),
        )
        .expect("screen-only Studio recording");
        for sequence in 1..=6 {
            recording
                .push_screen(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * VIDEO_DURATION_NS, VIDEO_DURATION_NS)
                        .expect("video timestamp"),
                    vec![7; 160 * 90 * 4],
                )
                .expect("screen original");
        }
        let artifact = recording.finish().expect("sealed screen original");
        assert_eq!(artifact.assets.len(), 1);
        assert_eq!(artifact.assets[0].track, TrackKind::Screen);
        assert_eq!(artifact.tracks.len(), 1);
    }
}
