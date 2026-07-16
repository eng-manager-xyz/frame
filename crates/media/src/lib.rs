use std::path::Path;

use gst::prelude::*;
use gstreamer as gst;
use thiserror::Error;

const REQUIRED_FACTORIES: &[&str] = &[
    "videotestsrc",
    "videoconvert",
    "vp8enc",
    "webmmux",
    "filesink",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub version: String,
    pub required_factories: Vec<&'static str>,
}

pub fn probe_runtime() -> Result<RuntimeInfo, MediaError> {
    gst::init().map_err(|error| MediaError::Initialization(error.to_string()))?;

    for factory in REQUIRED_FACTORIES {
        if gst::ElementFactory::find(factory).is_none() {
            return Err(MediaError::MissingPlugin((*factory).to_owned()));
        }
    }

    Ok(RuntimeInfo {
        version: gst::version_string().to_string(),
        required_factories: REQUIRED_FACTORIES.to_vec(),
    })
}

pub fn record_synthetic_webm(path: &Path) -> Result<(), MediaError> {
    probe_runtime()?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(MediaError::Output)?;
    }

    let element = gst::parse::launch(
        "videotestsrc num-buffers=60 pattern=ball ! videoconvert ! vp8enc deadline=1 ! webmmux ! filesink name=output",
    )
    .map_err(|error| MediaError::Pipeline(error.to_string()))?;
    let pipeline = element.downcast::<gst::Pipeline>().map_err(|_| {
        MediaError::Pipeline("pipeline description did not create a Pipeline".into())
    })?;
    let output = pipeline
        .by_name("output")
        .ok_or_else(|| MediaError::Pipeline("filesink was not created".into()))?;
    output.set_property("location", path);

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|error| MediaError::State(error.to_string()))?;

    let bus = pipeline
        .bus()
        .ok_or_else(|| MediaError::Pipeline("pipeline has no bus".into()))?;
    let result = match bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(15),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    ) {
        Some(message) => match message.view() {
            gst::MessageView::Eos(_) => Ok(()),
            gst::MessageView::Error(error) => Err(MediaError::Pipeline(format!(
                "{} ({:?})",
                error.error(),
                error.debug()
            ))),
            _ => Err(MediaError::Pipeline("unexpected terminal message".into())),
        },
        None => Err(MediaError::Timeout),
    };

    pipeline
        .set_state(gst::State::Null)
        .map_err(|error| MediaError::State(error.to_string()))?;
    result
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("could not initialize GStreamer: {0}")]
    Initialization(String),
    #[error("required GStreamer element is unavailable: {0}")]
    MissingPlugin(String),
    #[error("GStreamer pipeline failed: {0}")]
    Pipeline(String),
    #[error("GStreamer state change failed: {0}")]
    State(String),
    #[error("GStreamer smoke pipeline timed out")]
    Timeout,
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
}
