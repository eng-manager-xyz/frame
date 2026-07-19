use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

use gst::prelude::{Cast, ElementExt, GstBinExtManual, GstObjectExt, PluginFeatureExt};
use gstreamer as gst;

use crate::MediaError;

pub const RUNTIME_MANIFEST_VERSION: u16 = 6;
pub const MEDIA_APPLICATION_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MINIMUM_GSTREAMER_VERSION: (u32, u32, u32) = (1, 22, 0);
pub const MINIMUM_GSTREAMER_VERSION_TEXT: &str = "1.22.0";

const TRUSTED_PLUGIN_PATH_VARIABLE: &str = "GST_PLUGIN_SYSTEM_PATH_1_0";
const BUILD_GSTREAMER_PLUGIN_DIR: &str = env!("FRAME_BUILD_GSTREAMER_PLUGIN_DIR");
const FORBIDDEN_PLUGIN_ENVIRONMENT: [&str; 14] = [
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_REGISTRY",
    "GST_REGISTRY_1_0",
    "GST_REGISTRY_DISABLE",
    "GST_REGISTRY_UPDATE",
    "GST_REGISTRY_FORK",
    "GST_REGISTRY_MODE",
    "GST_REGISTRY_REUSE_PLUGIN_SCANNER",
    "GST_PLUGIN_LOADING_WHITELIST",
    "GST_PLUGIN_FEATURE_RANK",
];
const FORBIDDEN_LOADER_ENVIRONMENT: [&str; 11] = [
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LD_AUDIT",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_FRAMEWORK_PATH",
    "DYLD_ROOT_PATH",
    "DYLD_IMAGE_SUFFIX",
    "DYLD_SHARED_REGION",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeCapability {
    SyntheticSmoke,
    ThumbnailDecode,
    AppSourceBridge,
    AppSinkBridge,
    AudioMixing,
    AudioMetering,
    AudioNormalize,
    CameraComposition,
    InstantSegmentation,
    StudioPreview,
    DecodeAnalysis,
    Mp4Muxing,
    QuickTimeMuxing,
    SoftwareH264,
    SoftwareAac,
    SoftwareOpus,
    MediaTransform,
}

impl fmt::Display for RuntimeCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SyntheticSmoke => "synthetic_smoke",
            Self::ThumbnailDecode => "thumbnail_decode",
            Self::AppSourceBridge => "appsrc_bridge",
            Self::AppSinkBridge => "appsink_bridge",
            Self::AudioMixing => "audio_mixing",
            Self::AudioMetering => "audio_metering",
            Self::AudioNormalize => "audio_normalize",
            Self::CameraComposition => "camera_composition",
            Self::InstantSegmentation => "instant_segmentation",
            Self::StudioPreview => "studio_preview",
            Self::DecodeAnalysis => "decode_analysis",
            Self::Mp4Muxing => "mp4_muxing",
            Self::QuickTimeMuxing => "quicktime_muxing",
            Self::SoftwareH264 => "software_h264",
            Self::SoftwareAac => "software_aac",
            Self::SoftwareOpus => "software_opus",
            Self::MediaTransform => "media_transform",
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
        factory: "audiotestsrc",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "capsfilter",
        capability: RuntimeCapability::SyntheticSmoke,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::All,
    },
    FactorySpec {
        factory: "identity",
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
        factory: "filesrc",
        capability: RuntimeCapability::ThumbnailDecode,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "fdsink",
        capability: RuntimeCapability::AppSourceBridge,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "fdsrc",
        capability: RuntimeCapability::DecodeAnalysis,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "matroskademux",
        capability: RuntimeCapability::DecodeAnalysis,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "vp8dec",
        capability: RuntimeCapability::DecodeAnalysis,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "decodebin",
        capability: RuntimeCapability::ThumbnailDecode,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "videoscale",
        capability: RuntimeCapability::ThumbnailDecode,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "pngenc",
        capability: RuntimeCapability::ThumbnailDecode,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "fakesink",
        capability: RuntimeCapability::ThumbnailDecode,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
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
        requirement: FactoryRequirement::Required,
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
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "audioresample",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "audiomixer",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "volume",
        capability: RuntimeCapability::AudioMixing,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "level",
        capability: RuntimeCapability::AudioMetering,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "wavenc",
        capability: RuntimeCapability::AudioNormalize,
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
        capability: RuntimeCapability::QuickTimeMuxing,
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
        factory: "h264parse",
        capability: RuntimeCapability::SoftwareH264,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "aacparse",
        capability: RuntimeCapability::SoftwareAac,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "videorate",
        capability: RuntimeCapability::MediaTransform,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "jpegenc",
        capability: RuntimeCapability::MediaTransform,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "gifenc",
        capability: RuntimeCapability::MediaTransform,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "lamemp3enc",
        capability: RuntimeCapability::MediaTransform,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "concat",
        capability: RuntimeCapability::MediaTransform,
        requirement: FactoryRequirement::Optional,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "opusenc",
        capability: RuntimeCapability::SoftwareOpus,
        requirement: FactoryRequirement::Required,
        platform: PlatformScope::NativeDesktop,
    },
    FactorySpec {
        factory: "opusdec",
        capability: RuntimeCapability::DecodeAnalysis,
        requirement: FactoryRequirement::Required,
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

/// Proof that the build-time GStreamer root passed trust checks before Frame
/// constructs a pipeline. The private field prevents callers from fabricating
/// readiness after initializing GStreamer through an unaudited path.
#[derive(Debug)]
pub struct ReadyRuntime {
    info: RuntimeInfo,
}

impl ReadyRuntime {
    #[must_use]
    pub const fn info(&self) -> &RuntimeInfo {
        &self.info
    }
}

/// A pre-initialization launch decision for the native desktop executable.
///
/// The raw development/release binary may replace itself once with the exact
/// build-time plugin root. A release-mode macOS bundle may do the same only
/// while it remains beneath this checkout's canonical `target` directory;
/// copied or installed bundles require the app-relative runtime from issue 22.
pub enum DesktopRuntimeLaunchPlan {
    Ready,
    Reexec(DesktopRuntimeReexec),
}

/// The only environment mutation allowed for the raw desktop re-exec.
///
/// This type intentionally omits `Debug`: the canonical plugin root is an
/// internal launch detail and must not enter privacy-safe diagnostics.
pub struct DesktopRuntimeReexec {
    executable: PathBuf,
    plugin_root: PathBuf,
}

impl DesktopRuntimeReexec {
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Applies the audited environment to a command that replaces the current
    /// process. Callers must preserve the current argument vector.
    pub fn apply_to(&self, command: &mut Command) {
        for variable in FORBIDDEN_PLUGIN_ENVIRONMENT {
            command.env_remove(variable);
        }
        for variable in FORBIDDEN_LOADER_ENVIRONMENT {
            command.env_remove(variable);
        }
        command.env(TRUSTED_PLUGIN_PATH_VARIABLE, self.plugin_root.as_os_str());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryDiagnostic {
    pub factory: &'static str,
    pub capability: RuntimeCapability,
    pub requirement: FactoryRequirement,
    pub platform: PlatformScope,
    pub available: bool,
    pub trusted_provenance: bool,
    pub plugin_version: Option<String>,
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
    PluginSearchPathOverride(&'static str),
    LoaderEnvironmentOverride(&'static str),
    TrustedPluginPathRequired(&'static str),
    FactoryOutsideTrustedRoot(&'static str),
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
        if !self.is_ready() {
            return false;
        }
        let relevant: Vec<_> = self
            .factories
            .iter()
            .filter(|factory| factory.capability == capability)
            .collect();
        !relevant.is_empty()
            && relevant.iter().all(|factory| match factory.requirement {
                FactoryRequirement::Required | FactoryRequirement::Optional => {
                    factory.available && factory.trusted_provenance
                }
                FactoryRequirement::Prohibited => !factory.available,
            })
    }
}

/// Plans the trusted GStreamer environment before a macOS desktop process
/// calls `gst::init` or creates application threads.
///
/// A missing trusted path is recoverable only for a raw executable, or for a
/// release-mode application bundle still beneath this checkout's canonical
/// `target` directory. Explicit overrides remain errors. A copied or installed
/// application bundle must carry the signed, app-relative runtime tracked by
/// issue 22.
pub fn desktop_runtime_launch_plan() -> Result<DesktopRuntimeLaunchPlan, MediaError> {
    let issues = plugin_loading_environment_issues();
    let trusted_path_is_configured =
        std::env::var_os(TRUSTED_PLUGIN_PATH_VARIABLE).is_some_and(|value| !value.is_empty());
    let reported_executable = std::env::current_exe().map_err(|_| {
        MediaError::Initialization(
            "could not resolve the desktop executable before GStreamer initialization".into(),
        )
    })?;
    let application_bundle_context = is_macos_application_bundle_executable(&reported_executable);
    let executable = std::fs::canonicalize(&reported_executable).map_err(|_| {
        MediaError::Initialization(
            "could not resolve the desktop executable before GStreamer initialization".into(),
        )
    })?;
    let application_bundle_context =
        application_bundle_context || is_macos_application_bundle_executable(&executable);
    let target_root = application_bundle_context
        .then(canonical_workspace_target_root)
        .flatten();
    desktop_runtime_launch_plan_for(
        executable,
        application_bundle_context,
        trusted_path_is_configured,
        &issues,
        target_root.as_deref(),
    )
}

fn desktop_runtime_launch_plan_for(
    executable: PathBuf,
    application_bundle_context: bool,
    trusted_path_is_configured: bool,
    issues: &[DiagnosticIssue],
    canonical_target_root: Option<&Path>,
) -> Result<DesktopRuntimeLaunchPlan, MediaError> {
    if let Some(issue) = issues
        .iter()
        .find(|issue| !matches!(issue, DiagnosticIssue::TrustedPluginPathRequired(_)))
    {
        return Err(diagnostic_issue_error(issue));
    }

    if application_bundle_context
        && !canonical_target_root.is_some_and(|root| executable.starts_with(root))
    {
        return Err(MediaError::Initialization(
            "the macOS application bundle has no audited app-relative GStreamer runtime".into(),
        ));
    }

    if trusted_path_is_configured {
        if issues.is_empty() {
            Ok(DesktopRuntimeLaunchPlan::Ready)
        } else {
            Err(diagnostic_issue_error(&issues[0]))
        }
    } else {
        Ok(DesktopRuntimeLaunchPlan::Reexec(DesktopRuntimeReexec {
            executable,
            plugin_root: PathBuf::from(BUILD_GSTREAMER_PLUGIN_DIR),
        }))
    }
}

fn canonical_workspace_target_root() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent()?.parent()?;
    std::fs::canonicalize(workspace.join("target")).ok()
}

fn is_macos_application_bundle_executable(executable: &Path) -> bool {
    let Some(macos) = executable.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    let Some(application) = contents.parent() else {
        return false;
    };
    macos.file_name() == Some(OsStr::new("MacOS"))
        && contents.file_name() == Some(OsStr::new("Contents"))
        && application
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

/// Produces a privacy-safe doctor report containing only public runtime and
/// factory names. It can identify a blocked environment variable by its public
/// name, but never includes its value, filesystem paths, device labels, or
/// plugin registry paths.
#[must_use]
pub fn diagnose_runtime() -> RuntimeDiagnostics {
    let manifest = runtime_manifest();
    let search_path_issues = plugin_loading_environment_issues();
    if !search_path_issues.is_empty() {
        // Environment-controlled plugin paths are checked before gst::init(),
        // because initialization is itself capable of loading plugin code.
        return RuntimeDiagnostics {
            manifest_version: manifest.schema_version,
            runtime_version: None,
            factories: Vec::new(),
            issues: search_path_issues,
        };
    }
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
            let factory = gst::ElementFactory::find(spec.factory);
            let available = factory.is_some();
            let trusted_provenance = factory.as_ref().is_some_and(factory_has_trusted_provenance);
            let plugin_version = factory
                .as_ref()
                .filter(|_| trusted_provenance)
                .and_then(factory_plugin_version);
            match (spec.requirement, available) {
                (FactoryRequirement::Required, false) => {
                    issues.push(DiagnosticIssue::MissingRequiredFactory(spec.factory));
                }
                (FactoryRequirement::Prohibited, true) => {
                    issues.push(DiagnosticIssue::ProhibitedFactoryPresent(spec.factory));
                }
                _ => {}
            }
            if available && !trusted_provenance {
                issues.push(DiagnosticIssue::FactoryOutsideTrustedRoot(spec.factory));
            }
            FactoryDiagnostic {
                factory: spec.factory,
                capability: spec.capability,
                requirement: spec.requirement,
                platform: spec.platform,
                available,
                trusted_provenance,
                plugin_version,
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

pub fn prepare_runtime() -> Result<ReadyRuntime, MediaError> {
    let diagnostics = diagnose_runtime();
    if let Some(issue) = diagnostics.issues.first() {
        return Err(diagnostic_issue_error(issue));
    }
    let Some(version) = diagnostics.runtime_version.clone() else {
        return Err(MediaError::Initialization(
            "runtime initialization failed; run the media doctor".into(),
        ));
    };

    Ok(ReadyRuntime {
        info: RuntimeInfo {
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
        },
    })
}

fn diagnostic_issue_error(issue: &DiagnosticIssue) -> MediaError {
    match issue {
        DiagnosticIssue::InitializationFailed => {
            MediaError::Initialization("runtime initialization failed; run the media doctor".into())
        }
        DiagnosticIssue::RuntimeTooOld { required, found } => MediaError::RuntimeVersion {
            required,
            found: found.clone(),
        },
        DiagnosticIssue::MissingRequiredFactory(factory) => {
            MediaError::MissingPlugin((*factory).into())
        }
        DiagnosticIssue::ProhibitedFactoryPresent(factory) => MediaError::Initialization(format!(
            "prohibited GStreamer factory is installed: {factory}"
        )),
        DiagnosticIssue::PluginSearchPathOverride(variable) => MediaError::Initialization(format!(
            "untrusted GStreamer plugin search override is set in {variable}"
        )),
        DiagnosticIssue::LoaderEnvironmentOverride(variable) => MediaError::Initialization(
            format!("untrusted native loader override is set in {variable}"),
        ),
        DiagnosticIssue::TrustedPluginPathRequired(variable) => {
            MediaError::Initialization(format!(
                "the GStreamer plugin path in {variable} does not match the build-time runtime"
            ))
        }
        DiagnosticIssue::FactoryOutsideTrustedRoot(factory) => MediaError::Initialization(format!(
            "GStreamer factory {factory} is outside the build-time plugin root"
        )),
    }
}

pub fn probe_runtime() -> Result<RuntimeInfo, MediaError> {
    prepare_runtime().map(|runtime| runtime.info)
}

fn plugin_loading_environment_issues() -> Vec<DiagnosticIssue> {
    let mut issues: Vec<_> = FORBIDDEN_PLUGIN_ENVIRONMENT
        .into_iter()
        .filter(|variable| std::env::var_os(variable).is_some_and(|value| !value.is_empty()))
        .map(DiagnosticIssue::PluginSearchPathOverride)
        .collect();
    issues.extend(
        FORBIDDEN_LOADER_ENVIRONMENT
            .into_iter()
            .filter(|variable| std::env::var_os(variable).is_some_and(|value| !value.is_empty()))
            .map(DiagnosticIssue::LoaderEnvironmentOverride),
    );

    match std::env::var_os(TRUSTED_PLUGIN_PATH_VARIABLE) {
        Some(value) if trusted_plugin_path_matches(&value) => {}
        _ => issues.push(DiagnosticIssue::TrustedPluginPathRequired(
            TRUSTED_PLUGIN_PATH_VARIABLE,
        )),
    }
    issues
}

fn trusted_plugin_path_matches(value: &OsStr) -> bool {
    let configured = Path::new(value);
    if !configured.is_absolute() {
        return false;
    }
    std::fs::canonicalize(configured)
        .is_ok_and(|configured| configured == Path::new(BUILD_GSTREAMER_PLUGIN_DIR))
}

fn factory_has_trusted_provenance(factory: &gst::ElementFactory) -> bool {
    let Some(filename) = factory.plugin().and_then(|plugin| plugin.filename()) else {
        return false;
    };
    std::fs::canonicalize(filename)
        .is_ok_and(|filename| filename.starts_with(BUILD_GSTREAMER_PLUGIN_DIR))
}

fn factory_plugin_version(factory: &gst::ElementFactory) -> Option<String> {
    let version = factory.plugin()?.version();
    let version = version.as_str();
    (!version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'_')))
    .then(|| version.to_owned())
}

/// Verifies that every element currently instantiated in a graph is backed by
/// the canonical build-time plugin root. Call this after dynamic autoplugging
/// to cover decodebin-created children as well as Frame-authored elements.
#[must_use]
pub fn pipeline_has_trusted_factory_provenance(pipeline: &gst::Pipeline) -> bool {
    pipeline.iterate_recurse().into_iter().all(|element| {
        element.is_ok_and(|element| {
            element
                .factory()
                .as_ref()
                .is_some_and(factory_has_trusted_provenance)
                // Autopluggers may assemble anonymous/core structural bins.
                // They contain no plugin code of their own, and recursive
                // iteration still validates every executable child factory.
                || element.downcast_ref::<gst::Bin>().is_some()
        })
    })
}

/// Verifies the static, Frame-authored graph against the runtime manifest.
///
/// Call this before changing state. Dynamic descendants created later by an
/// audited autoplugger are the sole exception; those are checked separately by
/// [`pipeline_has_trusted_factory_provenance`] after autoplugging.
#[must_use]
pub fn pipeline_has_only_declared_authored_factories(pipeline: &gst::Pipeline) -> bool {
    let manifest = runtime_manifest();
    pipeline.children().iter().all(|element| {
        element.factory().is_some_and(|factory| {
            manifest
                .factories
                .iter()
                .any(|declared| declared.factory == factory.name().as_str())
        })
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
    fn decodebin_descendants_use_provenance_not_the_authored_allowlist() {
        prepare_runtime().expect("trusted test runtime");
        let pipeline = gst::parse::launch("filesrc ! decodebin ! fakesink")
            .expect("decode graph")
            .downcast::<gst::Pipeline>()
            .expect("pipeline");
        let descendants = pipeline
            .iterate_recurse()
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|element| element.factory())
            .map(|factory| factory.name().to_string())
            .collect::<Vec<_>>();

        assert!(descendants.iter().any(|factory| factory == "typefind"));
        assert!(
            !runtime_manifest()
                .factories
                .iter()
                .any(|declared| declared.factory == "typefind")
        );
        assert!(pipeline_has_only_declared_authored_factories(&pipeline));
        assert!(pipeline_has_trusted_factory_provenance(&pipeline));
    }

    #[test]
    fn diagnostics_never_include_registry_paths() {
        let diagnostics = diagnose_runtime();
        let rendered = format!("{diagnostics:?}");
        assert!(!rendered.contains("file://"));
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("/private/"));
    }

    #[test]
    fn plugin_path_issue_names_the_variable_not_its_value() {
        let issue = DiagnosticIssue::PluginSearchPathOverride("GST_PLUGIN_PATH");
        let rendered = format!("{issue:?}");
        assert!(rendered.contains("GST_PLUGIN_PATH"));
        assert!(!rendered.contains("/private/"));
    }

    #[test]
    fn loader_issue_names_the_variable_not_its_value() {
        let issue = DiagnosticIssue::LoaderEnvironmentOverride("LD_PRELOAD");
        let rendered = format!("{issue:?}");
        assert!(rendered.contains("LD_PRELOAD"));
        assert!(!rendered.contains("/private/"));
    }

    #[cfg(unix)]
    #[test]
    fn cargo_native_runner_clears_loader_overrides_at_the_executable_boundary() {
        for variable in FORBIDDEN_LOADER_ENVIRONMENT {
            assert!(
                std::env::var_os(variable).is_none_or(|value| value.is_empty()),
                "trusted Cargo runner did not clear {variable}"
            );
        }
    }

    #[test]
    fn build_time_plugin_directory_is_absolute_and_canonical() {
        let directory = Path::new(BUILD_GSTREAMER_PLUGIN_DIR);
        assert!(directory.is_absolute());
        assert_eq!(
            std::fs::canonicalize(directory).expect("canonical plugin directory"),
            directory
        );
    }

    #[test]
    fn raw_desktop_executable_is_not_mistaken_for_an_application_bundle() {
        assert!(!is_macos_application_bundle_executable(Path::new(
            "/workspace/frame/target/release/frame-desktop"
        )));
        assert!(is_macos_application_bundle_executable(Path::new(
            "/Applications/Frame.app/Contents/MacOS/frame-desktop"
        )));
        assert!(is_macos_application_bundle_executable(Path::new(
            "/Applications/Frame.APP/Contents/MacOS/frame-desktop"
        )));
    }

    #[test]
    fn desktop_launch_plan_has_no_marker_and_fails_closed_for_bundles() {
        let raw = PathBuf::from("/workspace/frame/target/release/frame-desktop");
        let bundled = PathBuf::from("/Applications/Frame.app/Contents/MacOS/frame-desktop");
        let target_root = Path::new("/workspace/frame/target");
        let local_bundle = PathBuf::from(
            "/workspace/frame/target/release/bundle/macos/Frame.app/Contents/MacOS/frame-desktop",
        );
        let missing = [DiagnosticIssue::TrustedPluginPathRequired(
            TRUSTED_PLUGIN_PATH_VARIABLE,
        )];

        assert!(matches!(
            desktop_runtime_launch_plan_for(raw.clone(), false, true, &[], None),
            Ok(DesktopRuntimeLaunchPlan::Ready)
        ));
        assert!(matches!(
            desktop_runtime_launch_plan_for(raw.clone(), false, false, &missing, None),
            Ok(DesktopRuntimeLaunchPlan::Reexec(_))
        ));
        assert!(matches!(
            desktop_runtime_launch_plan_for(raw, false, true, &missing, None),
            Err(MediaError::Initialization(_))
        ));
        assert!(matches!(
            desktop_runtime_launch_plan_for(bundled, true, true, &[], Some(target_root)),
            Err(MediaError::Initialization(_))
        ));
        assert!(matches!(
            desktop_runtime_launch_plan_for(local_bundle, true, false, &missing, Some(target_root)),
            Ok(DesktopRuntimeLaunchPlan::Reexec(_))
        ));
        assert!(matches!(
            desktop_runtime_launch_plan_for(
                PathBuf::from("/workspace/frame/target/release/frame-desktop"),
                false,
                false,
                &[DiagnosticIssue::LoaderEnvironmentOverride(
                    "DYLD_INSERT_LIBRARIES"
                )],
                None,
            ),
            Err(MediaError::Initialization(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_target_membership_rejects_traversal_and_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let target = temporary.path().join("target");
        let outside = temporary.path().join("outside/Frame.app/Contents/MacOS");
        std::fs::create_dir_all(&target).expect("target directory");
        std::fs::create_dir_all(&outside).expect("outside bundle directory");
        std::fs::write(outside.join("frame-desktop"), b"test").expect("test executable");
        symlink(
            temporary.path().join("outside/Frame.app"),
            target.join("Escaped.app"),
        )
        .expect("bundle symlink");
        let linked_macos = target.join("Linked.app/Contents/MacOS");
        std::fs::create_dir_all(&linked_macos).expect("linked bundle directory");
        let outside_raw = temporary.path().join("outside/frame-desktop");
        std::fs::write(&outside_raw, b"test").expect("outside raw executable");
        let linked_executable = linked_macos.join("frame-desktop");
        symlink(&outside_raw, &linked_executable).expect("executable symlink");

        let canonical_target = std::fs::canonicalize(&target).expect("canonical target");
        let through_symlink =
            std::fs::canonicalize(target.join("Escaped.app/Contents/MacOS/frame-desktop"))
                .expect("canonical symlink executable");
        let through_parent =
            std::fs::canonicalize(target.join("../outside/Frame.app/Contents/MacOS/frame-desktop"))
                .expect("canonical traversed executable");
        let missing = [DiagnosticIssue::TrustedPluginPathRequired(
            TRUSTED_PLUGIN_PATH_VARIABLE,
        )];

        for escaped in [through_symlink, through_parent] {
            assert!(is_macos_application_bundle_executable(&escaped));
            assert!(!escaped.starts_with(&canonical_target));
            assert!(matches!(
                desktop_runtime_launch_plan_for(
                    escaped,
                    true,
                    false,
                    &missing,
                    Some(&canonical_target),
                ),
                Err(MediaError::Initialization(_))
            ));
        }

        let canonical_linked =
            std::fs::canonicalize(&linked_executable).expect("canonical linked executable");
        assert!(is_macos_application_bundle_executable(&linked_executable));
        assert!(!is_macos_application_bundle_executable(&canonical_linked));
        assert!(matches!(
            desktop_runtime_launch_plan_for(
                canonical_linked,
                true,
                false,
                &missing,
                Some(&canonical_target),
            ),
            Err(MediaError::Initialization(_))
        ));
    }

    #[test]
    fn desktop_reexec_sets_only_the_canonical_path_and_clears_overrides() {
        let plan = DesktopRuntimeReexec {
            executable: PathBuf::from("/workspace/frame/target/release/frame-desktop"),
            plugin_root: PathBuf::from(BUILD_GSTREAMER_PLUGIN_DIR),
        };
        let mut command = Command::new(plan.executable());
        for variable in FORBIDDEN_PLUGIN_ENVIRONMENT {
            command.env(variable, "/tmp/hostile-plugin-path");
        }
        for variable in FORBIDDEN_LOADER_ENVIRONMENT {
            command.env(variable, "/tmp/hostile-loader-path");
        }

        plan.apply_to(&mut command);

        for variable in FORBIDDEN_PLUGIN_ENVIRONMENT
            .into_iter()
            .chain(FORBIDDEN_LOADER_ENVIRONMENT)
        {
            assert_eq!(
                command
                    .get_envs()
                    .find(|(name, _)| *name == OsStr::new(variable))
                    .and_then(|(_, value)| value),
                None,
                "{variable} must be removed"
            );
        }
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == OsStr::new(TRUSTED_PLUGIN_PATH_VARIABLE))
                .and_then(|(_, value)| value),
            Some(OsStr::new(BUILD_GSTREAMER_PLUGIN_DIR))
        );
    }

    #[test]
    fn arbitrary_plugin_directories_never_match_the_build_runtime() {
        assert!(!trusted_plugin_path_matches(OsStr::new("relative/plugins")));
        assert!(!trusted_plugin_path_matches(OsStr::new(
            "/tmp/frame-untrusted-gstreamer-plugins"
        )));
    }

    #[test]
    fn alternative_muxers_have_independent_capabilities() {
        let diagnostics = RuntimeDiagnostics {
            manifest_version: RUNTIME_MANIFEST_VERSION,
            runtime_version: Some("GStreamer test".into()),
            factories: vec![FactoryDiagnostic {
                factory: "mp4mux",
                capability: RuntimeCapability::Mp4Muxing,
                requirement: FactoryRequirement::Optional,
                platform: PlatformScope::NativeDesktop,
                available: true,
                trusted_provenance: true,
                plugin_version: Some("1.22.0".into()),
            }],
            issues: Vec::new(),
        };
        assert!(diagnostics.capability_available(RuntimeCapability::Mp4Muxing));
        assert!(!diagnostics.capability_available(RuntimeCapability::QuickTimeMuxing));
    }
}
