use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use frame_media::*;

const VIDEO_WIDTH: u32 = 160;
const VIDEO_HEIGHT: u32 = 90;
const VIDEO_FRAME_DURATION_NS: u64 = 33_333_333;
const AUDIO_FRAMES_PER_BUFFER: usize = 1_024;
const AUDIO_BUFFER_DURATION_NS: u64 = 21_333_333;
const MAXIMUM_TRACK_BYTES: u64 = 64 * 1024 * 1024;

fn project_id(marker: u8) -> StudioProjectId {
    StudioProjectId::from_csprng([marker; 16]).expect("project ID")
}

fn asset_id(marker: u8) -> StudioAssetId {
    StudioAssetId::from_csprng([marker; 16]).expect("asset ID")
}

fn operation_id(marker: u8) -> StudioOperationId {
    StudioOperationId::from_csprng([marker; 16]).expect("operation ID")
}

fn encoding(track: TrackKind) -> StudioAssetEncoding {
    match track {
        TrackKind::Screen | TrackKind::Camera => {
            StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
                width: VIDEO_WIDTH,
                height: VIDEO_HEIGHT,
                frame_rate: FrameRate {
                    numerator: 30,
                    denominator: 1,
                },
                pixel_format: PixelFormat::Bgra8,
            })
            .expect("video encoding")
        }
        TrackKind::Microphone | TrackKind::SystemAudio => {
            StudioAssetEncoding::recording_opus_webm(StudioAudioRawCaps {
                sample_rate: 48_000,
                channels: 2,
                sample_format: AudioSampleFormat::Float32,
            })
            .expect("audio encoding")
        }
    }
}

fn branch(marker: u8, track: TrackKind) -> IsolatedTrackBranch {
    IsolatedTrackBranch {
        track,
        asset_id: asset_id(marker),
        temporary_name: StudioSourceName::new(format!("track-{marker}.webm")).expect("source name"),
        source: match track {
            TrackKind::Screen => CaptureElementFamily::NativeScreenBridge,
            TrackKind::Camera => CaptureElementFamily::NativeCameraBridge,
            TrackKind::Microphone => CaptureElementFamily::NativeMicrophoneBridge,
            TrackKind::SystemAudio => CaptureElementFamily::NativeSystemAudioBridge,
        },
        encoder: match track {
            TrackKind::Screen | TrackKind::Camera => CaptureElementFamily::Vp8Encoder,
            TrackKind::Microphone | TrackKind::SystemAudio => CaptureElementFamily::OpusEncoder,
        },
        muxer: CaptureElementFamily::WebMMux,
        encoding: encoding(track),
        queue: BoundedMediaQueue {
            max_buffers: 128,
            max_bytes: 64 * 1024 * 1024,
            max_time_ns: 3_000_000_000,
        },
    }
}

fn graph(
    project_marker: u8,
    clock_marker: u8,
    tracks: &[(u8, TrackKind)],
) -> StudioRecordingGraphSpec {
    StudioRecordingGraphSpec::new(
        project_id(project_marker),
        operation_id(clock_marker),
        tracks
            .iter()
            .map(|(marker, track)| branch(*marker, *track))
            .collect(),
    )
    .expect("recording graph")
}

fn video_input(track: TrackKind, sequence: u64) -> NativeStudioInputBuffer {
    let pts_ns = (sequence - 1) * VIDEO_FRAME_DURATION_NS;
    NativeStudioInputBuffer::new(
        track,
        sequence,
        FrameTimestamp::new(pts_ns, VIDEO_FRAME_DURATION_NS).expect("video timestamp"),
        vec![
            u8::try_from(sequence % 255).expect("pixel marker");
            VIDEO_WIDTH as usize * VIDEO_HEIGHT as usize * 4
        ],
    )
    .expect("video input")
}

fn audio_input(track: TrackKind, sequence: u64, frequency_marker: f32) -> NativeStudioInputBuffer {
    let mut samples = Vec::with_capacity(AUDIO_FRAMES_PER_BUFFER * 2 * size_of::<f32>());
    for frame in 0..AUDIO_FRAMES_PER_BUFFER {
        let sample = ((frame as f32 / frequency_marker).sin() * 0.25).to_le_bytes();
        samples.extend_from_slice(&sample);
        samples.extend_from_slice(&sample);
    }
    NativeStudioInputBuffer::new(
        track,
        sequence,
        FrameTimestamp::new(
            (sequence - 1) * AUDIO_BUFFER_DURATION_NS,
            AUDIO_BUFFER_DURATION_NS,
        )
        .expect("audio timestamp"),
        samples,
    )
    .expect("audio input")
}

fn temporary_media_path(root: &Path, project_marker: u8, asset_marker: u8) -> PathBuf {
    root.join(format!("{project_marker:02x}").repeat(16))
        .join("temporary")
        .join(format!(
            "{}.media",
            format!("{asset_marker:02x}").repeat(16)
        ))
}

fn assert_webm(path: &Path) {
    let bytes = fs::read(path).expect("read isolated WebM");
    assert!(bytes.len() > 1_024);
    assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3]);
}

#[test]
fn invalid_input_terminalizes_the_exact_recording_graph() {
    let directory = tempfile::tempdir().expect("recording directory");
    let store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let graph = graph(5, 7, &[(6, TrackKind::Screen)]);
    let session =
        FilesystemStudioRecordingSession::begin(&store, graph.clone(), MAXIMUM_TRACK_BYTES)
            .expect("durable recording session");
    let mut recording =
        NativeStudioRecording::start(&graph, session).expect("native recording graph");
    let invalid = NativeStudioInputBuffer::new(
        TrackKind::Screen,
        1,
        FrameTimestamp::new(0, VIDEO_FRAME_DURATION_NS).expect("timestamp"),
        vec![0; 4],
    )
    .expect("structurally valid input");
    assert!(matches!(
        recording.push(invalid),
        Err(NativeStudioRecordingError::InvalidInput)
    ));
    assert_eq!(recording.state(), NativeStudioRecordingState::Failed);
    assert!(matches!(
        recording.push(video_input(TrackKind::Screen, 2)),
        Err(NativeStudioRecordingError::InvalidLifecycle)
    ));
    recording.abort().expect("confirmed Null");
}

#[test]
fn production_multitrack_graph_seals_four_independent_originals() {
    let directory = tempfile::tempdir().expect("recording directory");
    let mut store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let tracks = [
        (11, TrackKind::Screen),
        (12, TrackKind::Camera),
        (13, TrackKind::Microphone),
        (14, TrackKind::SystemAudio),
    ];
    let graph = graph(10, 15, &tracks);
    let session =
        FilesystemStudioRecordingSession::begin(&store, graph.clone(), MAXIMUM_TRACK_BYTES)
            .expect("durable recording session");
    let mut recording =
        NativeStudioRecording::start(&graph, session).expect("native recording graph");

    for sequence in 1..=60 {
        recording
            .push(video_input(TrackKind::Screen, sequence))
            .expect("screen buffer");
        recording
            .push(video_input(TrackKind::Camera, sequence))
            .expect("camera buffer");
    }
    for sequence in 1..=94 {
        recording
            .push(audio_input(TrackKind::Microphone, sequence, 7.0))
            .expect("microphone buffer");
        recording
            .push(audio_input(TrackKind::SystemAudio, sequence, 11.0))
            .expect("system-audio buffer");
    }

    let artifact = recording
        .finish(&CancellationToken::new())
        .expect("finished isolated originals");
    assert_eq!(artifact.assets.len(), 4);
    assert_eq!(artifact.tracks.len(), 4);
    assert!(
        artifact
            .tracks
            .iter()
            .all(|track| track.submitted_buffers > 0 && track.encoded_bytes > 1_024)
    );
    for (asset_marker, track) in tracks {
        let path = temporary_media_path(directory.path(), 10, asset_marker);
        assert_webm(&path);
        if matches!(track, TrackKind::Screen | TrackKind::Camera) {
            let preview =
                decode_studio_preview_frame(&path, Duration::ZERO, &CancellationToken::new())
                    .expect("decoded isolated video");
            assert_eq!((preview.width, preview.height), (320, 180));
        }
        let asset = artifact
            .assets
            .iter()
            .find(|asset| asset.track == track)
            .expect("track asset")
            .clone();
        let committed = commit_verified_temporary(
            &mut store,
            TempAssetCommitTicket::new(
                project_id(10),
                operation_id(asset_marker.saturating_add(32)),
                1,
                asset,
            )
            .expect("commit ticket"),
        )
        .expect("durable original");
        assert_eq!(committed.commit_state, AssetCommitState::DurableOriginal);
        assert_eq!(
            store
                .probe_original(project_id(10), committed.id)
                .expect("probe original"),
            Some(committed)
        );
    }
}

#[test]
fn interrupted_streamable_webm_is_recovered_without_flattening() {
    let directory = tempfile::tempdir().expect("recovery directory");
    let store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let graph = graph(20, 22, &[(21, TrackKind::Screen)]);
    let session =
        FilesystemStudioRecordingSession::begin(&store, graph.clone(), MAXIMUM_TRACK_BYTES)
            .expect("durable recording session");
    let mut recording =
        NativeStudioRecording::start(&graph, session).expect("native recording graph");
    for sequence in 1..=90 {
        recording
            .push(video_input(TrackKind::Screen, sequence))
            .expect("screen buffer");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while recording
        .status(TrackKind::Screen)
        .is_none_or(|status| status.encoded_bytes <= 1_024)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        recording
            .status(TrackKind::Screen)
            .is_some_and(|status| status.encoded_bytes > 1_024)
    );
    recording
        .abort()
        .expect("confirmed Null while preserving partial");

    let recovered =
        FilesystemStudioRecordingSession::recover(&store, graph.clone(), MAXIMUM_TRACK_BYTES)
            .expect("rehashed recording partial");
    assert!(matches!(
        NativeStudioRecording::start(&graph, recovered),
        Err(NativeStudioRecordingError::InvalidLifecycle)
    ));
    let recovered = FilesystemStudioRecordingSession::recover(&store, graph, MAXIMUM_TRACK_BYTES)
        .expect("reopen retained partial after rejected container append");
    let assets = recovered
        .finish(
            RationalTime::from_nanos(0),
            RationalTime::from_nanos(3_000_000_000),
        )
        .expect("sealed recovered partial");
    assert_eq!(assets.len(), 1);
    let path = temporary_media_path(directory.path(), 20, 21);
    assert_webm(&path);
    decode_studio_preview_frame(&path, Duration::ZERO, &CancellationToken::new())
        .expect("decoded recovered streamable WebM");
}
