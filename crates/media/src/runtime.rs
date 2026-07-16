use std::fmt;

use gstreamer as gst;

use crate::MediaError;

pub const RUNTIME_MANIFEST_VERSION: u16 = 1;
pub const MINIMUM_GSTREAMER_VERSION: (u32, u32, u32) = (1, 22, 0);
pub const MINIMUM_GSTREAMER_VERSION_TEXT: &str = "1.22.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeCapability {
    SyntheticSmoke,
    AppSourceBridge,
    AppSinkBridge,
    AudioMixing,
    CameraComposition,
    InstantSegmentation,
    StudioPreview,
    DecodeAnalysis,
    Mp4Muxing,
    SoftwareH264,
    SoftwareAac,
    SoftwareOpus,
}

impl fmt::Display for RuntimeCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SyntheticSmoke => "synthetic_smoke",
            Self::AppSourceBridge => "appsrc_bridge",
            Self::AppSinkBridge => "appsink_bridge",
            Self::AudioMixing => "audio_mixing",
            Self::CameraComposition => "camera_composition",
            Self::InstantSegmentation => "instant_segmentation",
            Self::StudioPreview => "studio_preview",
            Self::DecodeAnalysis => "decode_analysis",
            Self::Mp4Muxing => "mp4_muxing",
            Self::SoftwareH264 => "software_h264",
            Self::SoftwareAac => "software_aac",
            Self::SoftwareOpus => "software_opus",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryRequirement {
    Required,
    Optional,
    Prohibited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformScope {
    All,
    NativeDesktop,
    Linux,
    MacOs,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactorySpec {
    pub factory: &'static str,
    pub capability: RuntimeCapability,
    pub requirement: FactoryRequirement,
    pub platform: PlatformScope,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeManifest {
    pub schema_version: u16,
    pub minimum_gstreamer: (u32, u32, u32),
    pub factories: &'static [FactorySpec],
}

const FACTORIES: &[FactorySpec] = &[
    FactorySpec {
        factory: "videotestsrc",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "videoconvert",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "vp8enc",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "webmmux",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "filesink",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "appsrc",
        capability: RuntimeCapability::AppSourceBridge,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "queue",
        capability: RuntimeCapability::AppSourceBridge,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "appsink",
        capability: RuntimeCapability::AppSinkBridge,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "audioconvert",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "audioresample",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "audiomixer",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "compositor",
        capability: RuntimeCapability::CameraComposition,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "splitmuxsink",
        capability: RuntimeCapability::InstantSegmentation,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "playbin3",
        capability: RuntimeCapability::StudioPreview,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "decodebin3",
        capability: RuntimeCapability::DecodeAnalysis,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "mp4mux",
        capability: RuntimeCapability::Mp4Muxing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "qtmux",
        capability: RuntimeCapability::Mp4Muxing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "x264enc",
        capability: RuntimeCapability::SoftwareH264,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "avenc_aac",
        capability: RuntimeCapability::SoftwareAac,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "opusenc",
        capability: RuntimeCapability::SoftwareOpus,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
];

#[must_use]
pub const fn runtime_manifest() -> RuntimeManifest {
    RuntimeManifest {
        schema_version: RUNTIME_MANIFEST_VERSION,
        minimum_gstreamer: MINIMUM_GSTREAMER_VERSION,
        factories: FACTORIES,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub version: String,
    pub manifest_version: u16,
    pub required_factories: Vec<&'static str>,
    pub optional_factories_available: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryDiagnostic {
    pub factory: &'static str,
    pub capability: RuntimeCapability,
    pub requirement: FactoryRequirement,
    pub platform: PlatformScope,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticIssue {
    InitializationFailed,
    RuntimeTooOld {
        required: &'static str,
        found: String,
    },
    MissingRequiredFactory(&'static str),
    ProhibitedFactoryPresent(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    pub manifest_version: u16,
    pub runtime_version: Option<String>,
    pub factories: Vec<FactoryDiagnostic>,
    pub issues: Vec<DiagnosticIssue>,
}

impl RuntimeDiagnostics {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.runtime_version.is_some() && self.issues.is_empty()
    }

    #[must_use]
    pub fn capability_available(&self, capability: RuntimeCapability) -> bool {
        let relevant: Vec<_> = self
            .factories
            .iter()
            .filter(|factory| factory.capability == capability)
            .collect();
        !relevant.is_empty()
            && relevant.iter().all(|factory| match factory.requirement {
                FactoryRequirement::Required | FactoryRequirement::Optional => factory.available,
                FactoryRequirement::Prohibited => !factory.available,
            })
    }
}

/// Produces a privacy-safe doctor report containing only public runtime and
/// factory names. It never includes environment variables, filesystem paths,
/// device labels, or plugin registry paths.
#[must_use]
pub fn diagnose_runtime() -> RuntimeDiagnostics {
    let manifest = runtime_manifest();
    if gst::init().is_err() {
        return RuntimeDiagnostics {
            manifest_version: manifest.schema_version,
            runtime_version: None,
            factories: Vec::new(),
            issues: vec![DiagnosticIssue::InitializationFailed],
        };
    }

    let runtime_version = gst::version();
    let mut issues = Vec::new();
    if (runtime_version.0, runtime_version.1, runtime_version.2) < manifest.minimum_gstreamer {
        issues.push(DiagnosticIssue::RuntimeTooOld {
            required: MINIMUM_GSTREAMER_VERSION_TEXT,
            found: format!(
                "{}.{}.{}",
                runtime_version.0, runtime_version.1, runtime_version.2
            ),
        });
    }

    let factories = manifest
        .factories
        .iter()
        .map(|spec| {
            let available = gst::ElementFactory::find(spec.factory).is_some();
            match (spec.requirement, available) {
                (FactoryRequirement::Required, false) => {
                    issues.push(DiagnosticIssue::MissingRequiredFactory(spec.factory));
                }
                (FactoryRequirement::Prohibited, true) => {
                    issues.push(DiagnosticIssue::ProhibitedFactoryPresent(spec.factory));
                }
                _ => {}
            }
            FactoryDiagnostic {
                factory: spec.factory,
                capability: spec.capability,
                requirement: spec.requirement,
                platform: spec.platform,
                available,
            }
        })
        .collect();

    RuntimeDiagnostics {
        manifest_version: manifest.schema_version,
        runtime_version: Some(gst::version_string().to_string()),
        factories,
        issues,
    }
}

pub fn probe_runtime() -> Result<RuntimeInfo, MediaError> {
    let diagnostics = diagnose_runtime();
    let Some(version) = diagnostics.runtime_version.clone() else {
        return Err(MediaError::Initialization(
            "runtime initialization failed; run the media doctor".into(),
        ));
    };

    if let Some(issue) = diagnostics.issues.first() {
        return match issue {
            DiagnosticIssue::InitializationFailed => Err(MediaError::Initialization(
                "runtime initialization failed; run the media doctor".into(),
            )),
            DiagnosticIssue::RuntimeTooOld { required, found } => Err(MediaError::RuntimeVersion {
                required,
                found: found.clone(),
            }),
            DiagnosticIssue::MissingRequiredFactory(factory) => {
                Err(MediaError::MissingPlugin((*factory).into()))
            }
            DiagnosticIssue::ProhibitedFactoryPresent(factory) => Err(MediaError::Initialization(
                format!("prohibited GStreamer factory is installed: {factory}"),
            )),
        };
    }

    Ok(RuntimeInfo {
        version,
        manifest_version: diagnostics.manifest_version,
        required_factories: diagnostics
            .factories
            .iter()
            .filter(|factory| factory.requirement == FactoryRequirement::Required)
            .map(|factory| factory.factory)
            .collect(),
        optional_factories_available: diagnostics
            .factories
            .iter()
            .filter(|factory| {
                factory.requirement == FactoryRequirement::Optional && factory.available
            })
            .map(|factory| factory.factory)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_versioned_and_has_unique_factories() {
        let manifest = runtime_manifest();
        assert_ne!(manifest.schema_version, 0);
        let mut names: Vec<_> = manifest.factories.iter().map(|item| item.factory).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn diagnostics_never_include_registry_paths() {
        let diagnostics = diagnose_runtime();
        let rendered = format!("{diagnostics:?}");
        assert!(!rendered.contains("file://"));
        assert!(!rendered.contains("GST_PLUGIN_PATH"));
    }
}
