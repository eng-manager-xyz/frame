//! Native media primitives shared by Frame's desktop and media-worker runtimes.
//!
//! This crate deliberately keeps provider, UI, and database concerns out of the
//! GStreamer process. Its public types model bounded media lifecycle,
//! capture timing, recording recovery, executor routing, and conformance checks.

mod capture;
mod conformance;
mod instant;
mod jobs;
mod pipeline;
mod runtime;
mod studio;
mod supervisor;

use std::path::Path;

use gst::prelude::*;
use gstreamer as gst;
use thiserror::Error;

pub use capture::*;
pub use conformance::*;
pub use instant::*;
pub use jobs::*;
pub use pipeline::*;
pub use runtime::*;
pub use studio::*;
pub use supervisor::*;

/// Records the existing fixed-profile synthetic VP8/WebM smoke fixture.
///
/// The smoke remains intentionally native-only. Production profiles are chosen
/// through [`MediaRouter`], not by changing this small runtime diagnostic.
pub fn record_synthetic_webm(path: &Path) -> Result<(), MediaError> {
    record_synthetic_webm_with_cancel(path, &CancellationToken::new())
}

/// Records the synthetic smoke fixture while honoring cooperative cancellation.
pub fn record_synthetic_webm_with_cancel(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), MediaError> {
    record_synthetic_av_webm(path, cancellation).map(|_| ())
}

/// Records a fixed-profile synthetic VP8 + Opus WebM fixture and returns the complete,
/// privacy-safe supervisor report.
pub fn record_synthetic_av_webm(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<PipelineRunReport, MediaError> {
    let runtime = prepare_runtime()?;
    if cancellation.is_cancelled() {
        return Err(MediaError::Cancelled);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(MediaError::Output)?;
    }

    let element = gst::parse::launch(concat!(
        "webmmux name=mux streamable=false ! identity name=progress ",
        "! filesink name=output ",
        "videotestsrc num-buffers=60 is-live=false pattern=ball ",
        "! video/x-raw,width=320,height=180,framerate=30/1 ",
        "! queue max-size-buffers=64 max-size-bytes=0 max-size-time=0 leaky=no ",
        "! identity name=video_timing ",
        "! videoconvert ! vp8enc deadline=1 ",
        "! queue max-size-buffers=64 max-size-bytes=0 max-size-time=0 leaky=no ! mux. ",
        "audiotestsrc num-buffers=94 samplesperbuffer=1024 is-live=false wave=sine freq=440 ",
        "! audio/x-raw,format=F32LE,rate=48000,channels=1 ",
        "! queue max-size-buffers=128 max-size-bytes=0 max-size-time=0 leaky=no ",
        "! identity name=audio_timing ",
        "! audioconvert ! audioresample ! opusenc ",
        "! queue max-size-buffers=128 max-size-bytes=0 max-size-time=0 leaky=no ! mux."
    ))
    .map_err(|_| MediaError::Pipeline("could not construct audited synthetic A/V graph".into()))?;
    let pipeline = element.downcast::<gst::Pipeline>().map_err(|_| {
        MediaError::Pipeline("pipeline description did not create a Pipeline".into())
    })?;
    let output = pipeline
        .by_name("output")
        .ok_or_else(|| MediaError::Pipeline("filesink was not created".into()))?;
    output.set_property("location", path);

    let supervisor = PipelineSupervisor::new(
        &runtime,
        pipeline,
        "progress",
        PipelineCorrelationId::new("synthetic-av-smoke")?,
        SupervisorPolicy::default(),
    )?
    .with_av_timing_probes("audio_timing", "video_timing")?;
    let report = supervisor.run(cancellation)?;
    match (report.outcome, report.teardown) {
        (PipelineTerminalOutcome::Completed, PipelineTeardown::NullReached) => Ok(report),
        (PipelineTerminalOutcome::Cancelled, _) => {
            remove_partial_output(path);
            Err(MediaError::Cancelled)
        }
        (PipelineTerminalOutcome::Failed(fault), _) => {
            remove_partial_output(path);
            match fault.code {
                PipelineFaultCode::Timeout => Err(MediaError::Timeout),
                PipelineFaultCode::SinkBlocked => Err(MediaError::SinkBlocked),
                _ => Err(MediaError::Pipeline(fault.safe_message.into())),
            }
        }
        (PipelineTerminalOutcome::Completed, _) => {
            remove_partial_output(path);
            Err(MediaError::State(
                "pipeline did not confirm the Null state".into(),
            ))
        }
    }
}

fn remove_partial_output(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("could not initialize GStreamer: {0}")]
    Initialization(String),
    #[error("GStreamer {found} is older than required {required}")]
    RuntimeVersion {
        required: &'static str,
        found: String,
    },
    #[error("required GStreamer element is unavailable: {0}")]
    MissingPlugin(String),
    #[error("GStreamer pipeline failed: {0}")]
    Pipeline(String),
    #[error("GStreamer state change failed: {0}")]
    State(String),
    #[error("GStreamer smoke pipeline timed out")]
    Timeout,
    #[error("GStreamer pipeline stopped making output progress")]
    SinkBlocked,
    #[error("GStreamer operation was cancelled")]
    Cancelled,
    #[error("GStreamer supervisor rejected the pipeline: {0}")]
    Supervisor(#[from] SupervisorError),
    #[error("invalid pipeline lifecycle: {0}")]
    Lifecycle(#[source] LifecycleError),
    #[error("could not prepare output path: {0}")]
    Output(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_runtime_is_available() {
        let info = probe_runtime().expect("GStreamer runtime and smoke plugins");
        assert!(info.version.contains("GStreamer"));
        assert_eq!(info.manifest_version, RUNTIME_MANIFEST_VERSION);
    }

    #[test]
    fn synthetic_pipeline_writes_audio_and_video_media() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let output = directory.path().join("smoke.webm");
        let report = record_synthetic_av_webm(&output, &CancellationToken::new())
            .expect("record synthetic WebM");
        let bytes = std::fs::read(output).expect("output bytes");
        let size = bytes.len();
        assert!(
            size > 1_024,
            "expected a non-trivial media artifact, got {size} bytes"
        );
        assert!(
            bytes.windows(b"V_VP8".len()).any(|value| value == b"V_VP8"),
            "WebM is missing its VP8 track"
        );
        assert!(
            bytes
                .windows(b"A_OPUS".len())
                .any(|value| value == b"A_OPUS"),
            "WebM is missing its Opus track"
        );
        assert!(report.completed());
        assert!(
            report
                .diagnostics
                .factories
                .contains(&"audiotestsrc".into())
        );
        assert!(report.diagnostics.factories.contains(&"opusenc".into()));
        assert_eq!(report.diagnostics.queues.len(), 4);
        assert!(report.diagnostics.queues.iter().all(|queue| {
            queue.max_buffers > 0 || queue.max_bytes > 0 || queue.max_time_ns > 0
        }));
        let timing = report.diagnostics.av_timing.expect("A/V timing probes");
        assert!(timing.start_offset_ns.unsigned_abs() <= 1_000_000);
        assert!(timing.drift_ns.unsigned_abs() <= 25_000_000);
    }

    #[test]
    fn cancelled_smoke_does_not_start() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let output = directory.path().join("cancelled.webm");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            record_synthetic_webm_with_cancel(&output, &cancellation),
            Err(MediaError::Cancelled)
        ));
        assert!(!output.exists());
    }
}
