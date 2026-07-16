//! Native media primitives shared by Frame's desktop and media-worker runtimes.
//!
//! This crate deliberately keeps provider, UI, and database concerns out of the
//! GStreamer process.  Its public types model deterministic media lifecycle,
//! capture timing, recording recovery, executor routing, and conformance checks.

mod capture;
mod conformance;
mod instant;
mod jobs;
mod pipeline;
mod runtime;
mod studio;

use std::{path::Path, time::Instant};

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

/// Records the existing deterministic VP8/WebM smoke fixture.
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
    probe_runtime()?;
    if cancellation.is_cancelled() {
        return Err(MediaError::Cancelled);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(MediaError::Output)?;
    }

    let mut lifecycle = PipelineLifecycle::new();
    lifecycle
        .apply(PipelineCommand::Prepare)
        .map_err(MediaError::Lifecycle)?;

    let element = gst::parse::launch(
        "videotestsrc num-buffers=60 pattern=ball ! videoconvert ! vp8enc deadline=1 ! webmmux ! filesink name=output",
    )
    .map_err(|error| MediaError::Pipeline(redact_gstreamer_error(&error.to_string())))?;
    let pipeline = element.downcast::<gst::Pipeline>().map_err(|_| {
        MediaError::Pipeline("pipeline description did not create a Pipeline".into())
    })?;
    let output = pipeline
        .by_name("output")
        .ok_or_else(|| MediaError::Pipeline("filesink was not created".into()))?;
    output.set_property("location", path);

    let bus = pipeline
        .bus()
        .ok_or_else(|| MediaError::Pipeline("pipeline has no bus".into()))?;
    if let Err(error) = pipeline.set_state(gst::State::Playing) {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(MediaError::State(error.to_string()));
    }
    if let Err(error) = lifecycle.apply(PipelineCommand::Start) {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(MediaError::Lifecycle(error));
    }
    let started = Instant::now();
    let timeout = std::time::Duration::from_secs(15);
    let poll = gst::ClockTime::from_mseconds(50);

    let terminal_result = loop {
        if cancellation.is_cancelled() {
            let _ = lifecycle.apply(PipelineCommand::Cancel);
            break Err(MediaError::Cancelled);
        }
        if started.elapsed() >= timeout {
            let _ = lifecycle.apply(PipelineCommand::Fail(PipelineFault::timeout()));
            break Err(MediaError::Timeout);
        }

        let Some(message) =
            bus.timed_pop_filtered(poll, &[gst::MessageType::Eos, gst::MessageType::Error])
        else {
            continue;
        };

        match message.view() {
            gst::MessageView::Eos(_) => {
                if let Err(error) = lifecycle.apply(PipelineCommand::BeginFinalize) {
                    break Err(MediaError::Lifecycle(error));
                }
                if let Err(error) = lifecycle.apply(PipelineCommand::Complete) {
                    break Err(MediaError::Lifecycle(error));
                }
                break Ok(());
            }
            gst::MessageView::Error(error) => {
                let safe_message = redact_gstreamer_error(&error.error().to_string());
                let _ = lifecycle.apply(PipelineCommand::Fail(PipelineFault::pipeline()));
                break Err(MediaError::Pipeline(safe_message));
            }
            _ => continue,
        }
    };

    // Teardown is attempted on every terminal path so devices and files are not
    // retained after a timeout, cancellation, or bus error.
    let teardown_result = pipeline
        .set_state(gst::State::Null)
        .map_err(|error| MediaError::State(error.to_string()));

    match (terminal_result, teardown_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

fn redact_gstreamer_error(message: &str) -> String {
    // GStreamer errors can include a local file URI after a colon. Preserve the
    // useful error class without putting filesystem paths into diagnostics.
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return "unspecified GStreamer error".into();
    }
    trimmed
        .split("file://")
        .next()
        .unwrap_or("GStreamer pipeline error")
        .chars()
        .take(240)
        .collect()
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
    #[error("GStreamer operation was cancelled")]
    Cancelled,
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
    fn synthetic_pipeline_writes_media() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let output = directory.path().join("smoke.webm");
        record_synthetic_webm(&output).expect("record synthetic WebM");
        let size = std::fs::metadata(output).expect("output metadata").len();
        assert!(
            size > 1_024,
            "expected a non-trivial media artifact, got {size} bytes"
        );
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

    #[test]
    fn gstreamer_errors_drop_file_uris() {
        let redacted = redact_gstreamer_error("failed file:///Users/example/private.mov");
        assert_eq!(redacted, "failed ");
        assert!(!redacted.contains("private.mov"));
    }
}
