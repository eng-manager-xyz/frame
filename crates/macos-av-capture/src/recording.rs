//! Product-facing owner for one normalized macOS optional-input recording.

use std::time::Instant;

use frame_media::{
    AV_SETTINGS_VERSION, AudioSourceMixSettings, AvActionExecution, AvCaptureError,
    AvCaptureSession, AvCaptureSettingsV2, AvDeviceCatalog, AvDiagnostic, AvPipelineGraphSpec,
    AvRuntimePolicy, AvSessionId, AvSourceClass, AvSyncPolicy, BoundNativeAvBridge,
    DeviceSelectionV2, MonotonicTimeNs, NativeAvFailureCode, NativeAvGraphOutputSample,
    NativeAvGraphTeardown, NativeAvGstreamerGraph, NativeAvRuntime, NativeAvRuntimeError,
    NativeAvRuntimeOutcome, NativeAvRuntimeState, NativeAvSourceTeardown, NativeAvTermination,
    PermissionState,
};
use frame_platform_lifecycle::SystemPowerMonitor;
use thiserror::Error;

use crate::{MacOsDeviceAvBridge, MacOsDeviceAvBridgeCreateError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MacOsOptionalInputRequest {
    pub microphone: bool,
    pub system_audio: bool,
    pub camera: bool,
    pub camera_preview: bool,
}

impl MacOsOptionalInputRequest {
    #[must_use]
    pub const fn any_enabled(self) -> bool {
        self.microphone || self.system_audio || self.camera
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MacOsOptionalInputSelection {
    pub microphone: bool,
    pub system_audio: bool,
    pub camera: bool,
    pub camera_preview: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MacOsOptionalInputCatalog {
    pub revision: u64,
    pub microphones: u16,
    pub system_audio_sources: u16,
    pub cameras: u16,
}

#[derive(Debug, Error)]
pub enum MacOsOptionalInputError {
    #[error("macOS optional-input adapter creation failed")]
    Adapter(#[from] MacOsDeviceAvBridgeCreateError),
    #[error("macOS optional-input contract failed")]
    Capture(#[from] AvCaptureError),
    #[error("macOS optional-input graph construction failed")]
    Graph,
    #[error("macOS optional-input runtime failed")]
    Runtime(#[from] NativeAvRuntimeError),
    #[error("macOS optional-input native operation failed: {0:?}")]
    Native(NativeAvFailureCode),
    #[error("macOS optional-input count exceeded its product bound")]
    CatalogBound,
}

pub struct MacOsOptionalInputStart {
    pub selection: MacOsOptionalInputSelection,
    pub recording: Option<MacOsOptionalInputRecording>,
}

#[derive(Debug)]
pub struct MacOsOptionalInputPoll {
    pub output_samples: Vec<NativeAvGraphOutputSample>,
    pub diagnostics: Vec<AvDiagnostic>,
    pub termination: Option<NativeAvTermination>,
}

impl std::fmt::Debug for MacOsOptionalInputStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacOsOptionalInputStart")
            .field("selection", &self.selection)
            .field("recording", &self.recording.as_ref().map(|_| "<active>"))
            .finish()
    }
}

pub struct MacOsOptionalInputRecording {
    runtime: Option<NativeAvRuntime<MacOsDeviceAvBridge>>,
    inactive_state: NativeAvRuntimeState,
    master_origin: Instant,
    selection: MacOsOptionalInputSelection,
}

impl std::fmt::Debug for MacOsOptionalInputRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacOsOptionalInputRecording")
            .field("state", &self.state())
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

pub fn enumerate_optional_inputs(
    installation_secret: [u8; 32],
    power: &SystemPowerMonitor,
    session_id: AvSessionId,
) -> Result<MacOsOptionalInputCatalog, MacOsOptionalInputError> {
    let bridge = MacOsDeviceAvBridge::new(installation_secret, power)?;
    let mut bound = BoundNativeAvBridge::new(bridge, session_id)?;
    let _session = AvCaptureSession::new(bound.claim_session()?);
    let catalog = bound.enumerate()?;
    Ok(MacOsOptionalInputCatalog {
        revision: catalog.revision(),
        microphones: count_sources(&catalog, AvSourceClass::Microphone)?,
        system_audio_sources: count_sources(&catalog, AvSourceClass::SystemAudio)?,
        cameras: count_sources(&catalog, AvSourceClass::Camera)?,
    })
}

impl MacOsOptionalInputRecording {
    pub fn start(
        installation_secret: [u8; 32],
        power: &SystemPowerMonitor,
        session_id: AvSessionId,
        master_origin: Instant,
        request: MacOsOptionalInputRequest,
    ) -> Result<MacOsOptionalInputStart, MacOsOptionalInputError> {
        if !request.any_enabled() {
            return Ok(MacOsOptionalInputStart {
                selection: MacOsOptionalInputSelection::default(),
                recording: None,
            });
        }
        let bridge =
            MacOsDeviceAvBridge::new_with_master_origin(installation_secret, power, master_origin)?;
        let mut bound = BoundNativeAvBridge::new(bridge, session_id)?;
        let mut session = AvCaptureSession::new(bound.claim_session()?);
        let capabilities = bound.capabilities()?;
        let mut catalog = bound.enumerate()?;

        let microphone = authorize_requested_source(
            &mut bound,
            &mut session,
            &mut catalog,
            AvSourceClass::Microphone,
            request.microphone,
        )?;
        let system_audio = authorize_requested_source(
            &mut bound,
            &mut session,
            &mut catalog,
            AvSourceClass::SystemAudio,
            request.system_audio,
        )?;
        let camera = authorize_requested_source(
            &mut bound,
            &mut session,
            &mut catalog,
            AvSourceClass::Camera,
            request.camera,
        )?;
        let selection = MacOsOptionalInputSelection {
            microphone,
            system_audio,
            camera,
            camera_preview: camera && request.camera_preview,
        };
        if !(microphone || system_audio || camera) {
            return Ok(MacOsOptionalInputStart {
                selection,
                recording: Some(Self {
                    runtime: None,
                    inactive_state: NativeAvRuntimeState::Playing,
                    master_origin,
                    selection,
                }),
            });
        }

        let settings = AvCaptureSettingsV2 {
            version: AV_SETTINGS_VERSION,
            microphone: selection_for(&catalog, AvSourceClass::Microphone, microphone)?,
            system_audio: selection_for(&catalog, AvSourceClass::SystemAudio, system_audio)?,
            camera: selection_for(&catalog, AvSourceClass::Camera, camera)?,
        };
        let graph_spec =
            AvPipelineGraphSpec::negotiate(&catalog, settings, selection.camera_preview)?;
        let action =
            session.request_start(capabilities, catalog, settings, selection.camera_preview)?;
        complete_action(&mut session, &mut bound, action)?;
        let graph = NativeAvGstreamerGraph::build_recording(&graph_spec)
            .map_err(|_| MacOsOptionalInputError::Graph)?;
        let runtime = NativeAvRuntime::attach(
            bound,
            session,
            graph,
            AvSyncPolicy::default(),
            AvRuntimePolicy::default(),
        )?;
        Ok(MacOsOptionalInputStart {
            selection,
            recording: Some(Self {
                runtime: Some(runtime),
                inactive_state: NativeAvRuntimeState::Playing,
                master_origin,
                selection,
            }),
        })
    }

    #[must_use]
    pub const fn selection(&self) -> MacOsOptionalInputSelection {
        self.selection
    }

    #[must_use]
    pub fn state(&self) -> NativeAvRuntimeState {
        self.runtime
            .as_ref()
            .map_or(self.inactive_state, NativeAvRuntime::state)
    }

    #[must_use]
    pub fn master_now(&self) -> MonotonicTimeNs {
        monotonic_now(self.master_origin)
    }

    pub fn poll(&mut self) -> Result<MacOsOptionalInputPoll, MacOsOptionalInputError> {
        if let Some(runtime) = self.runtime.as_mut() {
            let now = monotonic_now(self.master_origin);
            let report = runtime.poll(now)?;
            return Ok(MacOsOptionalInputPoll {
                output_samples: report.output_samples,
                diagnostics: report.diagnostics,
                termination: report.termination,
            });
        }
        if self.inactive_state == NativeAvRuntimeState::EosRequested {
            self.inactive_state = NativeAvRuntimeState::NullConfirmed;
            return Ok(MacOsOptionalInputPoll {
                output_samples: Vec::new(),
                diagnostics: Vec::new(),
                termination: Some(NativeAvTermination {
                    outcome: NativeAvRuntimeOutcome::Completed,
                    source_teardown: NativeAvSourceTeardown::Confirmed,
                    graph_teardown: NativeAvGraphTeardown::NullReached,
                }),
            });
        }
        Ok(MacOsOptionalInputPoll {
            output_samples: Vec::new(),
            diagnostics: Vec::new(),
            termination: None,
        })
    }

    pub fn set_audio_mix(
        &mut self,
        class: AvSourceClass,
        settings: AudioSourceMixSettings,
    ) -> Result<(), MacOsOptionalInputError> {
        self.runtime
            .as_mut()
            .ok_or(NativeAvRuntimeError::InvalidTransition)?
            .set_audio_mix(class, settings)?;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), MacOsOptionalInputError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.pause(monotonic_now(self.master_origin))?;
        } else if self.inactive_state == NativeAvRuntimeState::Playing {
            self.inactive_state = NativeAvRuntimeState::Paused;
        } else {
            return Err(NativeAvRuntimeError::InvalidTransition.into());
        }
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), MacOsOptionalInputError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.resume(monotonic_now(self.master_origin))?;
        } else if self.inactive_state == NativeAvRuntimeState::Paused {
            self.inactive_state = NativeAvRuntimeState::Playing;
        } else {
            return Err(NativeAvRuntimeError::InvalidTransition.into());
        }
        Ok(())
    }

    pub fn request_stop(&mut self) -> Result<(), MacOsOptionalInputError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.request_stop(monotonic_now(self.master_origin))?;
        } else if matches!(
            self.inactive_state,
            NativeAvRuntimeState::Playing | NativeAvRuntimeState::Paused
        ) {
            self.inactive_state = NativeAvRuntimeState::EosRequested;
        } else {
            return Err(NativeAvRuntimeError::InvalidTransition.into());
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<NativeAvTermination, MacOsOptionalInputError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.cancel().map_err(Into::into)
        } else {
            self.inactive_state = NativeAvRuntimeState::NullConfirmed;
            Ok(NativeAvTermination {
                outcome: NativeAvRuntimeOutcome::Cancelled,
                source_teardown: NativeAvSourceTeardown::Confirmed,
                graph_teardown: NativeAvGraphTeardown::NullReached,
            })
        }
    }
}

fn monotonic_now(master_origin: Instant) -> MonotonicTimeNs {
    MonotonicTimeNs::new(u64::try_from(master_origin.elapsed().as_nanos()).unwrap_or(u64::MAX))
}

fn authorize_requested_source(
    bound: &mut BoundNativeAvBridge<MacOsDeviceAvBridge>,
    session: &mut AvCaptureSession,
    catalog: &mut AvDeviceCatalog,
    class: AvSourceClass,
    requested: bool,
) -> Result<bool, MacOsOptionalInputError> {
    if !requested {
        return Ok(false);
    }
    if !catalog
        .devices()
        .iter()
        .any(|device| device.class() == class && device.is_default())
    {
        return Ok(false);
    }
    if catalog
        .devices()
        .iter()
        .find(|device| device.class() == class && device.is_default())
        .is_some_and(|device| device.permission() == PermissionState::Granted)
    {
        return Ok(true);
    }
    let action = session.request_permission(class)?;
    match action.execute_source(session, bound)? {
        AvActionExecution::Acknowledged(acknowledgement) => {
            session.complete(acknowledgement)?;
        }
        AvActionExecution::Failed(failure) => {
            let failure = session.complete_failure(failure)?;
            return match failure.code {
                NativeAvFailureCode::PermissionDenied
                | NativeAvFailureCode::PermissionRestricted
                | NativeAvFailureCode::SourceUnavailable => Ok(false),
                code => Err(MacOsOptionalInputError::Native(code)),
            };
        }
    }
    *catalog = bound.enumerate()?;
    Ok(catalog
        .devices()
        .iter()
        .find(|device| device.class() == class && device.is_default())
        .is_some_and(|device| device.permission() == PermissionState::Granted))
}

fn selection_for(
    catalog: &AvDeviceCatalog,
    class: AvSourceClass,
    enabled: bool,
) -> Result<DeviceSelectionV2, MacOsOptionalInputError> {
    if !enabled {
        return Ok(DeviceSelectionV2::Disabled);
    }
    let device = catalog
        .devices()
        .iter()
        .find(|device| device.class() == class && device.is_default())
        .ok_or(MacOsOptionalInputError::Graph)?;
    let format = device
        .formats()
        .first()
        .copied()
        .ok_or(MacOsOptionalInputError::Graph)?;
    format.validate_for(class)?;
    Ok(DeviceSelectionV2::FollowDefault {
        format,
        allow_default_changes: false,
        confirmed_id: Some(device.id()),
    })
}

fn complete_action(
    session: &mut AvCaptureSession,
    bound: &mut BoundNativeAvBridge<MacOsDeviceAvBridge>,
    action: frame_media::AvSessionAction,
) -> Result<(), MacOsOptionalInputError> {
    match action.execute_source(session, bound)? {
        AvActionExecution::Acknowledged(acknowledgement) => {
            session.complete(acknowledgement)?;
            Ok(())
        }
        AvActionExecution::Failed(failure) => {
            let failure = session.complete_failure(failure)?;
            Err(MacOsOptionalInputError::Native(failure.code))
        }
    }
}

fn count_sources(
    catalog: &AvDeviceCatalog,
    class: AvSourceClass,
) -> Result<u16, MacOsOptionalInputError> {
    u16::try_from(
        catalog
            .devices()
            .iter()
            .filter(|device| device.class() == class)
            .count(),
    )
    .map_err(|_| MacOsOptionalInputError::CatalogBound)
}
