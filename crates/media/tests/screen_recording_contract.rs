use std::{
    thread,
    time::{Duration, Instant},
};

use frame_media::{
    BgraScreenFrame, CancellationToken, ColorSpace, FrameMemory, FrameTimestamp, PixelFormat,
    ScreenRecording, ScreenRecordingError, ScreenRecordingSpec, VideoFrameSpec,
    export_screen_recording_webm,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAME_DURATION_NS: u64 = 33_333_333;
const INGRESS_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn wait_for_ingress_capacity(recording: &ScreenRecording) {
    let deadline = Instant::now() + INGRESS_DRAIN_TIMEOUT;
    while recording.ingress_status().at_capacity {
        assert!(
            Instant::now() < deadline,
            "bounded appsrc queue did not drain before the test deadline"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn recording_spec() -> ScreenRecordingSpec {
    ScreenRecordingSpec::new(VideoFrameSpec {
        width: WIDTH,
        height: HEIGHT,
        pixel_format: PixelFormat::Bgra8,
        color_space: ColorSpace::Srgb,
        nominal_frame_duration_ns: FRAME_DURATION_NS,
        memory: FrameMemory::Cpu,
    })
    .expect("valid BGRA recording spec")
}

fn synthetic_bgra(index: u64) -> Vec<u8> {
    let mut pixels = vec![0_u8; usize::try_from(WIDTH * HEIGHT * 4).expect("bounded fixture")];
    let (bgra_pixels, remainder) = pixels.as_chunks_mut::<4>();
    assert!(remainder.is_empty());
    for (pixel_index, pixel) in bgra_pixels.iter_mut().enumerate() {
        let x = u8::try_from(pixel_index % usize::try_from(WIDTH).expect("fixture width"))
            .unwrap_or(u8::MAX);
        let y = u8::try_from(pixel_index / usize::try_from(WIDTH).expect("fixture width"))
            .unwrap_or(u8::MAX);
        pixel[0] = x.wrapping_add(u8::try_from(index).unwrap_or(u8::MAX));
        pixel[1] = y;
        pixel[2] = 255_u8.wrapping_sub(x);
        pixel[3] = 255;
    }
    pixels
}

#[test]
fn owned_appsrc_graph_records_and_exports_playable_webm() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let source = directory.path().join("screen.webm");
    let mut recording =
        ScreenRecording::start(&source, recording_spec()).expect("start owned appsrc graph");

    for index in 0_u64..30 {
        let status = recording
            .push_frame(
                BgraScreenFrame::new(
                    index + 1,
                    FrameTimestamp {
                        pts_ns: index * FRAME_DURATION_NS,
                        duration_ns: FRAME_DURATION_NS,
                        discontinuity: index == 15,
                    },
                    synthetic_bgra(index),
                )
                .expect("valid synthetic frame"),
            )
            .expect("non-blocking appsrc submission");
        assert_eq!(status.submitted_frames, index + 1);
        assert!(status.queued_frames <= recording.spec().ingress_max_frames());
        assert!(status.queued_bytes <= recording.spec().ingress_max_bytes());
        assert!(status.queued_time_ns <= recording.spec().ingress_max_time_ns());
        wait_for_ingress_capacity(&recording);
    }

    recording.end_of_stream().expect("appsrc EOS");
    recording.end_of_stream().expect("idempotent appsrc EOS");
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                31,
                FrameTimestamp::new(30 * FRAME_DURATION_NS, FRAME_DURATION_NS)
                    .expect("valid timestamp"),
                synthetic_bgra(30),
            )
            .expect("valid post-EOS frame")
        ),
        Err(ScreenRecordingError::InvalidLifecycle)
    ));

    let artifact = recording
        .finish(&CancellationToken::new())
        .expect("verified playable recording");
    assert_eq!(artifact.path, source);
    assert_eq!(artifact.submitted_frames, 30);
    assert_eq!(artifact.encoded_frames, artifact.submitted_frames);
    assert_eq!(artifact.first_pts_ns, 0);
    assert_eq!(
        artifact.end_pts_ns - artifact.first_pts_ns,
        artifact.encoded_duration_ns
    );
    assert!(
        artifact
            .encoded_duration_ns
            .abs_diff(30 * FRAME_DURATION_NS)
            <= 2_000_000
    );
    assert!(artifact.bytes > 1_024);
    assert_eq!(artifact.sha256.len(), 64);

    let export_path = directory.path().join("screen-export.webm");
    let export =
        export_screen_recording_webm(&artifact.path, &export_path, &CancellationToken::new())
            .expect("verified real WebM export");
    assert_eq!(export.path, export_path);
    assert!(export.playable_container_marker);
    assert!(export.bytes > 1_024);
}

#[test]
fn discontinuity_rebases_rolled_back_pts_to_a_continuous_output_timeline() {
    const LONG_RUNNING_PTS_NS: u64 = 3 * 60 * 60 * 1_000_000_000;

    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("rebased-discontinuity.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");

    recording
        .push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(LONG_RUNNING_PTS_NS, FRAME_DURATION_NS)
                    .expect("long-running timestamp"),
                synthetic_bgra(0),
            )
            .expect("first frame"),
        )
        .expect("submit first frame");
    let status = recording
        .push_frame(
            BgraScreenFrame::new(
                2,
                FrameTimestamp {
                    pts_ns: 0,
                    duration_ns: FRAME_DURATION_NS,
                    discontinuity: true,
                },
                synthetic_bgra(1),
            )
            .expect("rolled-back discontinuity frame"),
        )
        .expect("discontinuity permits a rebased timestamp");
    assert_eq!(status.submitted_frames, 2);
    assert_eq!(recording.submitted_frames(), 2);
    recording
        .push_frame(
            BgraScreenFrame::new(
                3,
                FrameTimestamp::new(FRAME_DURATION_NS, FRAME_DURATION_NS)
                    .expect("next raw segment timestamp"),
                synthetic_bgra(2),
            )
            .expect("post-discontinuity frame"),
        )
        .expect("submit post-discontinuity frame");

    recording.end_of_stream().expect("appsrc EOS");
    let artifact = recording
        .finish(&CancellationToken::new())
        .expect("verified continuous recording");
    assert_eq!(artifact.submitted_frames, 3);
    assert_eq!(artifact.encoded_frames, 3);
    assert_eq!(artifact.first_pts_ns, LONG_RUNNING_PTS_NS);
    assert!(artifact.encoded_duration_ns.abs_diff(3 * FRAME_DURATION_NS) <= 2_000_000);
}

#[test]
fn discontinuity_does_not_relax_sequence_order_or_unmarked_pts_rollback() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("unmarked-rollback.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    recording
        .push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("first timestamp"),
                synthetic_bgra(0),
            )
            .expect("first frame"),
        )
        .expect("submit first frame");
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                2,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("rolled-back timestamp"),
                synthetic_bgra(1),
            )
            .expect("rolled-back frame")
        ),
        Err(ScreenRecordingError::NonMonotonicFrame)
    ));
    recording.abort().expect("confirmed Null teardown");

    let output = directory.path().join("duplicate-sequence.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    recording
        .push_frame(
            BgraScreenFrame::new(
                7,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("first timestamp"),
                synthetic_bgra(0),
            )
            .expect("first frame"),
        )
        .expect("submit first frame");
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                7,
                FrameTimestamp {
                    pts_ns: 0,
                    duration_ns: FRAME_DURATION_NS,
                    discontinuity: true,
                },
                synthetic_bgra(1),
            )
            .expect("duplicate-sequence discontinuity frame")
        ),
        Err(ScreenRecordingError::NonMonotonicFrame)
    ));
    recording.abort().expect("confirmed Null teardown");
}

#[test]
fn cancelled_finish_removes_partial_output() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("cancelled.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    recording
        .push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("valid timestamp"),
                synthetic_bgra(0),
            )
            .expect("valid synthetic frame"),
        )
        .expect("submit frame");
    recording.end_of_stream().expect("appsrc EOS");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        recording.finish(&cancellation),
        Err(ScreenRecordingError::Cancelled)
    ));
    assert!(!output.exists());
}

#[test]
fn zero_frame_finish_confirms_teardown_and_removes_partial_output() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("zero-frame.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    recording.end_of_stream().expect("zero-frame EOS");
    assert!(matches!(
        recording.finish(&CancellationToken::new()),
        Err(ScreenRecordingError::InvalidLifecycle)
    ));
    assert!(!output.exists());
}

#[test]
fn frame_and_output_boundaries_fail_closed() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("existing.webm");
    std::fs::write(&output, b"do not replace").expect("existing output fixture");
    assert!(matches!(
        ScreenRecording::start(&output, recording_spec()),
        Err(ScreenRecordingError::OutputExists)
    ));
    assert_eq!(
        std::fs::read(&output).expect("existing output preserved"),
        b"do not replace"
    );

    let output = directory.path().join("bad-frame.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("valid timestamp"),
                vec![0; 4],
            )
            .expect("structurally valid frame")
        ),
        Err(ScreenRecordingError::InvalidFrame)
    ));
    assert!(matches!(
        recording.end_of_stream(),
        Err(ScreenRecordingError::InvalidFrame)
    ));
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("valid timestamp"),
                synthetic_bgra(0),
            )
            .expect("valid frame")
        ),
        Err(ScreenRecordingError::InvalidFrame)
    ));
    drop(recording);
    assert!(!output.exists());

    let output = directory.path().join("invalid-sequence.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    assert!(matches!(
        recording.push_frame(
            BgraScreenFrame::new(
                u64::MAX,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("valid timestamp"),
                synthetic_bgra(0),
            )
            .expect("structurally valid frame")
        ),
        Err(ScreenRecordingError::InvalidFrame)
    ));
    assert!(matches!(
        recording.end_of_stream(),
        Err(ScreenRecordingError::InvalidFrame)
    ));
    drop(recording);
    assert!(!output.exists());
}

#[test]
fn destination_created_during_recording_is_preserved_and_commit_fails_closed() {
    let directory = tempfile::tempdir().expect("private temporary directory");
    let output = directory.path().join("raced.webm");
    let mut recording =
        ScreenRecording::start(&output, recording_spec()).expect("start owned appsrc graph");
    std::fs::write(&output, b"concurrent owner").expect("concurrent destination fixture");
    recording
        .push_frame(
            BgraScreenFrame::new(
                1,
                FrameTimestamp::new(0, FRAME_DURATION_NS).expect("valid timestamp"),
                synthetic_bgra(0),
            )
            .expect("valid frame"),
        )
        .expect("submit frame");
    recording.end_of_stream().expect("appsrc EOS");
    assert!(matches!(
        recording.finish(&CancellationToken::new()),
        Err(ScreenRecordingError::OutputExists)
    ));
    assert_eq!(
        std::fs::read(output).expect("concurrent destination preserved"),
        b"concurrent owner"
    );
}

#[cfg(unix)]
#[test]
fn preopened_descriptor_is_the_only_recording_and_verification_target() {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let directory = tempfile::tempdir().expect("private temporary directory");
    let staging = directory.path().join("descriptor-staging.webm");
    let presentation = directory.path().join("descriptor-published.webm");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staging)
        .expect("private preopened file");
    let mut recording = ScreenRecording::start_preopened(&presentation, file, recording_spec())
        .expect("start descriptor-backed graph");

    for index in 0_u64..3 {
        recording
            .push_frame(
                BgraScreenFrame::new(
                    index + 1,
                    FrameTimestamp::new(index * FRAME_DURATION_NS, FRAME_DURATION_NS)
                        .expect("valid timestamp"),
                    synthetic_bgra(index),
                )
                .expect("valid descriptor fixture frame"),
            )
            .expect("submit descriptor fixture frame");
    }
    recording.end_of_stream().expect("descriptor graph EOS");
    let artifact = recording
        .finish(&CancellationToken::new())
        .expect("descriptor-backed recording verifies");

    assert_eq!(artifact.path, presentation);
    assert!(!presentation.exists());
    assert_eq!(
        std::fs::metadata(staging)
            .expect("staging descriptor remains caller-owned")
            .len(),
        artifact.bytes
    );
    assert_eq!(artifact.sha256.len(), 64);
}
