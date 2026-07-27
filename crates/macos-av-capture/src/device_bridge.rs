//! Bounded macOS microphone and camera capture behind `NativeAvBridge`.
//!
//! GStreamer owns the platform device providers (`osxaudiosrc` and
//! `avfvideosrc`). This module never exposes labels or provider identifiers:
//! device identity is HMAC-derived, buffers are copied into bounded owned
//! payloads, and only the provider-neutral contract crosses the crate boundary.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use frame_media::{
    AV_CAPTURE_CONTRACT_VERSION, AvAdapterInstanceId, AvBufferLease, AvCaptureError,
    AvControlEventStamp, AvDeviceCatalog, AvDeviceDescriptor, AvDeviceGeneration, AvDeviceId,
    AvFormat, AvNativeRequest, AvOperationTicket, AvOwnerBinding, AvPayloadBody,
    AvPipelineGraphSpec, AvSessionClaimTicket, AvSourceCallTicket, AvSourceClass, AvSourceStamp,
    AvTerminalPostcondition, AvTerminalReconcileTicket, CalibrationSample, CameraFormat,
    CatalogChangeReason, LatencyConfidence, MonotonicTimeNs, NativeAvAcknowledgement,
    NativeAvBridge, NativeAvBridgeCapabilities, NativeAvBuffer, NativeAvBufferTiming,
    NativeAvCalibrationBatch, NativeAvEvent, NativeAvFailure, NativeAvFailureCode,
    NativeRouteClass, NativeTimestampKind, PermissionState, PixelFormat, SourceLatency,
};
use frame_platform_lifecycle::{SystemPowerEvent, SystemPowerMonitorError};

#[cfg(target_os = "macos")]
use frame_media::{
    FactoryRequirement, PlatformScope, RuntimeCapability,
    pipeline_has_only_declared_authored_factories, pipeline_has_trusted_factory_provenance,
    prepare_runtime, runtime_manifest,
};
#[cfg(target_os = "macos")]
use frame_platform_lifecycle::{SystemPowerCursor, SystemPowerMonitor};
#[cfg(target_os = "macos")]
use gst::prelude::*;
#[cfg(target_os = "macos")]
use gstreamer as gst;
#[cfg(target_os = "macos")]
use gstreamer_app as gst_app;
#[cfg(target_os = "macos")]
use ring::hmac;
#[cfg(target_os = "macos")]
use thiserror::Error;

use crate::SYSTEM_AUDIO_FORMAT;

const MICROPHONE_FORMAT: frame_media::AudioFormat = SYSTEM_AUDIO_FORMAT;
const CAMERA_FORMAT: CameraFormat = CameraFormat {
    width: 1280,
    height: 720,
    frame_rate_numerator: 30,
    frame_rate_denominator: 1,
    pixel_format: PixelFormat::Bgra8,
};
const STARTUP_CALIBRATION_SAMPLES: usize = 5;
const STARTUP_CALIBRATION_TIMEOUT: Duration = Duration::from_millis(750);
const CALIBRATION_IDLE_POLL: Duration = Duration::from_millis(2);
const PERMISSION_PROBE_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(target_os = "macos")]
const ADAPTER_ID_DOMAIN: &[u8] = b"frame/macos-device-av-adapter/v1\0";
#[cfg(target_os = "macos")]
const MICROPHONE_DEVICE_ID_DOMAIN: &[u8] = b"frame/macos-microphone-device/v1\0";
#[cfg(target_os = "macos")]
const CAMERA_DEVICE_ID_DOMAIN: &[u8] = b"frame/macos-camera-device/v1\0";
#[cfg(target_os = "macos")]
const SOURCE_STATE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);
#[cfg(target_os = "macos")]
const PERMISSION_SAMPLE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(30);
#[cfg(target_os = "macos")]
const SOURCE_QUEUE_BUFFERS: u32 = 3;

#[cfg(target_os = "macos")]
#[derive(Debug, Error)]
pub enum MacOsDeviceAvBridgeCreateError {
    #[error("the macOS microphone/camera adapter identity could not be derived")]
    AdapterIdentity,
    #[error("the audited GStreamer runtime is unavailable")]
    Runtime,
    #[error("the macOS GStreamer device monitor is unavailable")]
    DeviceMonitor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputDevice {
    id: AvDeviceId,
    generation: AvDeviceGeneration,
    class: AvSourceClass,
    is_default: bool,
    permission: PermissionState,
    route: NativeRouteClass,
    formats: Vec<AvFormat>,
}

impl InputDevice {
    fn descriptor(&self) -> Result<AvDeviceDescriptor, NativeAvFailure> {
        AvDeviceDescriptor::new(
            self.id,
            self.generation,
            self.class,
            self.is_default,
            self.permission,
            self.route,
            NativeTimestampKind::HostMonotonic,
            self.formats.clone(),
        )
        .map_err(contract_failure)
    }
}

#[derive(Debug)]
struct InputBuffer {
    format: AvFormat,
    source_pts_ns: u64,
    arrival_ns: u64,
    duration_ns: u64,
    discontinuity: bool,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFailure {
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "constructed by the macOS device backend")
    )]
    PermissionDenied,
    SourceUnavailable,
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "constructed by the macOS device backend")
    )]
    Busy,
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "constructed by the macOS device backend")
    )]
    Timeout,
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "constructed by the macOS device backend")
    )]
    FormatChanged,
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "constructed by the macOS device backend")
    )]
    BackendFault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputStopFailure {
    failure: InputFailure,
    teardown_confirmed: bool,
}

trait DeviceInputs {
    fn devices(&mut self) -> Result<Vec<InputDevice>, InputFailure>;
    fn permission(&mut self, class: AvSourceClass) -> Result<PermissionState, InputFailure>;
    fn request_permission(
        &mut self,
        class: AvSourceClass,
        timeout: Duration,
    ) -> Result<PermissionState, InputFailure>;
    fn start(
        &mut self,
        class: AvSourceClass,
        device: AvDeviceId,
        format: AvFormat,
        master_origin: Instant,
    ) -> Result<(), InputFailure>;
    fn poll(&mut self, class: AvSourceClass) -> Result<Option<InputBuffer>, InputFailure>;
    fn stop_and_drain(
        &mut self,
        class: AvSourceClass,
    ) -> Result<Vec<InputBuffer>, InputStopFailure>;
    fn is_running(&self, class: AvSourceClass) -> bool;
    fn poll_catalog_change(&mut self) -> Result<Option<CatalogChangeReason>, InputFailure>;
}

trait PowerEvents {
    fn poll_power(&mut self) -> Result<Option<SystemPowerEvent>, SystemPowerMonitorError>;
}

#[cfg(target_os = "macos")]
struct NativePowerEvents(SystemPowerCursor);

#[cfg(target_os = "macos")]
impl PowerEvents for NativePowerEvents {
    fn poll_power(&mut self) -> Result<Option<SystemPowerEvent>, SystemPowerMonitorError> {
        self.0.poll()
    }
}

struct OwnedInputLease {
    bytes: Option<Vec<u8>>,
}

impl AvBufferLease for OwnedInputLease {
    fn retained_bytes(&self) -> u64 {
        self.bytes
            .as_ref()
            .and_then(|bytes| u64::try_from(bytes.len()).ok())
            .unwrap_or(u64::MAX)
    }

    fn take_payload(&mut self) -> Option<AvPayloadBody> {
        self.bytes.take().map(AvPayloadBody::Bytes)
    }

    fn release(self: Box<Self>) {}
}

#[derive(Debug)]
struct BufferedInput {
    input: InputBuffer,
    source_pts_ns: u64,
}

#[derive(Debug)]
struct ActiveInput {
    stamp: AvSourceStamp,
    device: AvDeviceId,
    generation: AvDeviceGeneration,
    format: AvFormat,
    source_origin_ns: Option<u64>,
    output_sequence: u64,
    buffered: VecDeque<BufferedInput>,
    calibration: Option<NativeAvCalibrationBatch>,
}

struct DeviceInputBridge<S, P> {
    inputs: S,
    power: P,
    adapter: AvAdapterInstanceId,
    binding: Option<AvOwnerBinding>,
    catalog_revision: u64,
    control_sequence: u64,
    permissions: BTreeMap<AvSourceClass, PermissionState>,
    next_permission_probe: Instant,
    active: BTreeMap<AvSourceClass, ActiveInput>,
    poll_cursor: usize,
    suspended: bool,
    applied_terminal: Option<frame_media::AvTerminalId>,
}

impl<S: DeviceInputs, P: PowerEvents> DeviceInputBridge<S, P> {
    fn new(mut inputs: S, power: P, adapter: AvAdapterInstanceId) -> Result<Self, NativeAvFailure> {
        let mut permissions = BTreeMap::new();
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            permissions.insert(class, inputs.permission(class).map_err(map_input_failure)?);
        }
        Ok(Self {
            inputs,
            power,
            adapter,
            binding: None,
            catalog_revision: 1,
            control_sequence: 0,
            permissions,
            next_permission_probe: Instant::now(),
            active: BTreeMap::new(),
            poll_cursor: 0,
            suspended: false,
            applied_terminal: None,
        })
    }

    fn require_binding(&self, binding: AvOwnerBinding) -> Result<(), NativeAvFailure> {
        if self.binding == Some(binding) {
            Ok(())
        } else {
            Err(backend_fault(false))
        }
    }

    fn catalog(&mut self) -> Result<AvDeviceCatalog, NativeAvFailure> {
        let devices = self.inputs.devices().map_err(map_input_failure)?;
        let descriptors = devices
            .iter()
            .map(InputDevice::descriptor)
            .collect::<Result<Vec<_>, _>>()?;
        AvDeviceCatalog::new(self.adapter, self.catalog_revision, descriptors)
            .map_err(contract_failure)
    }

    fn observe_permission(
        &mut self,
        class: AvSourceClass,
        permission: PermissionState,
    ) -> Result<bool, NativeAvFailure> {
        if self.permissions.get(&class).copied() == Some(permission) {
            return Ok(false);
        }
        self.permissions.insert(class, permission);
        self.bump_catalog_revision()?;
        Ok(true)
    }

    fn bump_catalog_revision(&mut self) -> Result<(), NativeAvFailure> {
        self.catalog_revision = self
            .catalog_revision
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        Ok(())
    }

    fn validate_graph(
        &mut self,
        ticket: &AvOperationTicket,
        graph: &AvPipelineGraphSpec,
    ) -> Result<Vec<(AvSourceStamp, AvDeviceId, AvDeviceGeneration, AvFormat)>, NativeAvFailure>
    {
        if graph.sources.len() != ticket.stamps().len() {
            return Err(capability_changed());
        }
        let devices = self.inputs.devices().map_err(map_input_failure)?;
        let mut plans = Vec::with_capacity(graph.sources.len());
        for source in &graph.sources {
            if !matches!(
                source.class,
                AvSourceClass::Microphone | AvSourceClass::Camera
            ) {
                return Err(capability_changed());
            }
            let stamp = ticket
                .stamps()
                .iter()
                .find(|stamp| stamp.class() == source.class)
                .copied()
                .ok_or_else(capability_changed)?;
            if stamp.generation() != source.generation {
                return Err(capability_changed());
            }
            let format = match source.input_caps {
                frame_media::ExactCapsSpec::Audio(caps)
                    if source.class == AvSourceClass::Microphone =>
                {
                    AvFormat::Audio(caps.format)
                }
                frame_media::ExactCapsSpec::Camera(caps)
                    if source.class == AvSourceClass::Camera =>
                {
                    AvFormat::Camera(caps.format)
                }
                _ => return Err(capability_changed()),
            };
            let device = devices
                .iter()
                .find(|device| device.class == source.class && device.id == source.device)
                .ok_or_else(capability_changed)?;
            if device.generation != source.generation
                || device.permission != PermissionState::Granted
                || !device.formats.contains(&format)
            {
                return Err(capability_changed());
            }
            plans.push((stamp, source.device, source.generation, format));
        }
        Ok(plans)
    }

    fn start_plans(
        &mut self,
        plans: Vec<(AvSourceStamp, AvDeviceId, AvDeviceGeneration, AvFormat)>,
    ) -> Result<(), NativeAvFailure> {
        self.stop_all(true)?;
        let master_origin = Instant::now();
        for (stamp, device, generation, format) in plans {
            let class = stamp.class();
            if let Err(error) = self
                .inputs
                .start(class, device, format, master_origin)
                .map_err(map_input_failure)
            {
                let teardown = self.stop_all(true);
                return teardown.and(Err(error));
            }
            self.active.insert(
                class,
                ActiveInput {
                    stamp,
                    device,
                    generation,
                    format,
                    source_origin_ns: None,
                    output_sequence: 0,
                    buffered: VecDeque::new(),
                    calibration: None,
                },
            );
        }
        self.poll_cursor = 0;
        self.suspended = false;
        Ok(())
    }

    fn resume_sources(&mut self, stamps: &[AvSourceStamp]) -> Result<(), NativeAvFailure> {
        if stamps.len() != self.active.len() {
            return Err(capability_changed());
        }
        let plans = self
            .active
            .values()
            .map(|active| {
                let stamp = stamps
                    .iter()
                    .find(|stamp| stamp.class() == active.stamp.class())
                    .copied()
                    .ok_or_else(capability_changed)?;
                if stamp.generation() != active.generation {
                    return Err(capability_changed());
                }
                Ok((stamp, active.device, active.generation, active.format))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.start_plans(plans)
    }

    fn halt_all(&mut self) -> Result<(), NativeAvFailure> {
        let classes = self.active.keys().copied().collect::<Vec<_>>();
        let mut first_failure = None;
        for class in classes {
            match self.inputs.stop_and_drain(class) {
                Ok(tail) => {
                    if let Some(active) = self.active.get_mut(&class) {
                        for input in tail {
                            if let Err(error) = buffer_input(active, input) {
                                first_failure.get_or_insert(error);
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    let failure = if error.teardown_confirmed {
                        map_input_failure(error.failure)
                    } else {
                        NativeAvFailure {
                            code: NativeAvFailureCode::Timeout,
                            retryable: true,
                        }
                    };
                    first_failure.get_or_insert(failure);
                }
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    fn stop_all(&mut self, clear_active: bool) -> Result<(), NativeAvFailure> {
        let stopped = self.halt_all();
        if clear_active
            && self
                .active
                .keys()
                .all(|class| !self.inputs.is_running(*class))
        {
            self.active.clear();
        } else if !clear_active {
            for active in self.active.values_mut() {
                active.source_origin_ns = None;
                active.output_sequence = 0;
                active.buffered.clear();
                active.calibration = None;
            }
        }
        stopped
    }

    fn collect_calibration(
        &mut self,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        let class = stamp.class();
        {
            let active = self.active.get(&class).ok_or_else(capability_changed)?;
            if active.stamp != stamp || !self.inputs.is_running(class) {
                return Err(capability_changed());
            }
            if let Some(batch) = &active.calibration {
                if batch.stamp() == stamp {
                    return Ok(batch.clone());
                }
                return Err(capability_changed());
            }
        }
        let deadline = Instant::now()
            .checked_add(STARTUP_CALIBRATION_TIMEOUT)
            .ok_or_else(|| backend_fault(false))?;
        loop {
            if self
                .active
                .get(&class)
                .is_some_and(|active| active.buffered.len() >= STARTUP_CALIBRATION_SAMPLES)
            {
                break;
            }
            match self.inputs.poll(class).map_err(map_input_failure)? {
                Some(input) => {
                    let active = self.active.get_mut(&class).ok_or_else(capability_changed)?;
                    buffer_input(active, input)?;
                }
                None if Instant::now() < deadline => {
                    std::thread::park_timeout(
                        CALIBRATION_IDLE_POLL
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                None => return Err(timeout_failure()),
            }
        }
        let active = self.active.get_mut(&class).ok_or_else(capability_changed)?;
        let samples = active
            .buffered
            .iter()
            .take(STARTUP_CALIBRATION_SAMPLES)
            .map(|buffered| CalibrationSample {
                master_arrival: MonotonicTimeNs::new(buffered.input.arrival_ns),
                source_pts_ns: buffered.source_pts_ns,
                latency: SourceLatency {
                    reported_ns: 0,
                    confidence: LatencyConfidence::Unknown,
                },
            })
            .collect();
        let batch = NativeAvCalibrationBatch::new(stamp, samples).map_err(contract_failure)?;
        for _ in 0..STARTUP_CALIBRATION_SAMPLES {
            active.buffered.pop_front();
        }
        active.calibration = Some(batch.clone());
        Ok(batch)
    }

    fn next_buffer_event(
        &mut self,
        class: AvSourceClass,
    ) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let Some(active) = self.active.get_mut(&class) else {
            return Ok(None);
        };
        if self.suspended || !self.inputs.is_running(class) {
            return Ok(None);
        }
        let buffered = if let Some(buffered) = active.buffered.pop_front() {
            Some(buffered)
        } else {
            self.inputs
                .poll(class)
                .map_err(map_input_failure)?
                .map(|input| {
                    let origin = *active.source_origin_ns.get_or_insert(input.source_pts_ns);
                    let source_pts_ns = input
                        .source_pts_ns
                        .checked_sub(origin)
                        .ok_or_else(|| backend_fault(false))?;
                    Ok(BufferedInput {
                        input,
                        source_pts_ns,
                    })
                })
                .transpose()?
        };
        let Some(buffered) = buffered else {
            return Ok(None);
        };
        active.output_sequence = active
            .output_sequence
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        let timing = NativeAvBufferTiming {
            sequence: active.output_sequence,
            source_pts_ns: buffered.source_pts_ns,
            duration_ns: buffered.input.duration_ns,
            arrival: MonotonicTimeNs::new(buffered.input.arrival_ns),
            latency: SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Unknown,
            },
            discontinuity: buffered.input.discontinuity,
        };
        let buffer = NativeAvBuffer::new(
            active.stamp,
            timing,
            buffered.input.format,
            Box::new(OwnedInputLease {
                bytes: Some(buffered.input.bytes),
            }),
        )
        .map_err(contract_failure)?;
        Ok(Some(NativeAvEvent::Buffer(buffer)))
    }

    fn next_control_stamp(&mut self) -> Result<AvControlEventStamp, NativeAvFailure> {
        let owner = self.binding.ok_or_else(|| backend_fault(false))?;
        self.control_sequence = self
            .control_sequence
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        AvControlEventStamp::new(owner, self.catalog_revision, self.control_sequence)
            .map_err(contract_failure)
    }

    fn poll_lifecycle(&mut self) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        match self.power.poll_power().map_err(|_| backend_fault(false))? {
            Some(SystemPowerEvent::WillSleep) => {
                self.stop_all(false)?;
                self.suspended = true;
                Ok(Some(NativeAvEvent::Sleep))
            }
            Some(SystemPowerEvent::DidWake) => Ok(Some(NativeAvEvent::Wake)),
            None => Ok(None),
        }
    }

    fn poll_catalog(&mut self) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let Some(reason) = self
            .inputs
            .poll_catalog_change()
            .map_err(map_input_failure)?
        else {
            return Ok(None);
        };
        self.bump_catalog_revision()?;
        let catalog = self.catalog()?;
        Ok(Some(NativeAvEvent::CatalogChanged {
            stamp: self.next_control_stamp()?,
            catalog,
            reason,
        }))
    }

    fn poll_permission(&mut self) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let now = Instant::now();
        if now < self.next_permission_probe {
            return Ok(None);
        }
        self.next_permission_probe = now
            .checked_add(PERMISSION_PROBE_INTERVAL)
            .ok_or_else(|| backend_fault(false))?;
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            let permission = self.inputs.permission(class).map_err(map_input_failure)?;
            if !self.observe_permission(class, permission)? {
                continue;
            }
            if matches!(
                permission,
                PermissionState::Denied | PermissionState::Restricted | PermissionState::Revoked
            ) && self.inputs.is_running(class)
            {
                self.inputs
                    .stop_and_drain(class)
                    .map_err(|error| map_input_failure(error.failure))?;
            }
            return Ok(Some(NativeAvEvent::PermissionChanged {
                stamp: self.next_control_stamp()?,
                class,
                state: permission,
            }));
        }
        Ok(None)
    }

    fn poll_round_robin(&mut self) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let classes = self.active.keys().copied().collect::<Vec<_>>();
        if classes.is_empty() {
            return Ok(None);
        }
        let start = self.poll_cursor % classes.len();
        for offset in 0..classes.len() {
            let index = (start + offset) % classes.len();
            if let Some(event) = self.next_buffer_event(classes[index])? {
                self.poll_cursor = (index + 1) % classes.len();
                return Ok(Some(event));
            }
        }
        self.poll_cursor = (start + 1) % classes.len();
        Ok(None)
    }
}

fn buffer_input(active: &mut ActiveInput, input: InputBuffer) -> Result<(), NativeAvFailure> {
    if input.format != active.format {
        return Err(NativeAvFailure {
            code: NativeAvFailureCode::FormatChanged,
            retryable: true,
        });
    }
    let origin = *active.source_origin_ns.get_or_insert(input.source_pts_ns);
    let source_pts_ns = input
        .source_pts_ns
        .checked_sub(origin)
        .ok_or_else(|| backend_fault(false))?;
    active.buffered.push_back(BufferedInput {
        input,
        source_pts_ns,
    });
    Ok(())
}

impl<S: DeviceInputs, P: PowerEvents> NativeAvBridge for DeviceInputBridge<S, P> {
    fn adapter_instance(&self) -> AvAdapterInstanceId {
        self.adapter
    }

    fn bind(&mut self, ticket: AvSessionClaimTicket) -> Result<AvOwnerBinding, NativeAvFailure> {
        if self.binding.is_some() {
            return Err(NativeAvFailure {
                code: NativeAvFailureCode::Busy,
                retryable: false,
            });
        }
        let binding = ticket.accept();
        if binding.adapter() != self.adapter {
            return Err(backend_fault(false));
        }
        self.binding = Some(binding);
        Ok(binding)
    }

    fn capabilities(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<NativeAvBridgeCapabilities, NativeAvFailure> {
        self.require_binding(ticket.binding())?;
        Ok(NativeAvBridgeCapabilities {
            contract_version: AV_CAPTURE_CONTRACT_VERSION,
            adapter: self.adapter,
            permission_prompt: true,
            hotplug_events: true,
            default_change_events: true,
            sleep_wake_events: true,
            bounded_nonblocking_ingress: true,
            explicit_timestamps: true,
            discontinuity_signaling: true,
            latency_reporting: true,
        })
    }

    fn enumerate(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<AvDeviceCatalog, NativeAvFailure> {
        self.require_binding(ticket.binding())?;
        self.catalog()
    }

    fn startup_calibration(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        self.require_binding(ticket.binding())?;
        self.collect_calibration(stamp)
    }

    fn reconcile_terminal(
        &mut self,
        ticket: AvTerminalReconcileTicket,
    ) -> Result<AvTerminalPostcondition, NativeAvFailure> {
        self.require_binding(ticket.owner())?;
        Ok(if self.applied_terminal == Some(ticket.terminal_id()) {
            AvTerminalPostcondition::Applied {
                terminal_id: ticket.terminal_id(),
            }
        } else {
            AvTerminalPostcondition::NotApplied
        })
    }

    fn execute(
        &mut self,
        ticket: AvOperationTicket,
        request: &AvNativeRequest,
    ) -> Result<NativeAvAcknowledgement, NativeAvFailure> {
        self.require_binding(ticket.owner())?;
        if ticket.kind() != request.kind() {
            return Err(backend_fault(false));
        }
        match request {
            AvNativeRequest::RequestPermission(class)
                if matches!(class, AvSourceClass::Microphone | AvSourceClass::Camera) =>
            {
                let permission = self
                    .inputs
                    .request_permission(*class, ticket.native_timeout())
                    .map_err(map_input_failure)?;
                self.observe_permission(*class, permission)?;
                if permission != PermissionState::Granted {
                    return Err(NativeAvFailure {
                        code: NativeAvFailureCode::PermissionDenied,
                        retryable: false,
                    });
                }
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::RequestPermission(_) => Err(NativeAvFailure {
                code: NativeAvFailureCode::SourceUnavailable,
                retryable: false,
            }),
            AvNativeRequest::Start(graph) | AvNativeRequest::Reconfigure(graph) => {
                let plans = self.validate_graph(&ticket, graph)?;
                self.start_plans(plans)?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Pause => {
                self.stop_all(false)?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Resume => {
                self.resume_sources(ticket.stamps())?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Stop | AvNativeRequest::Cancel => {
                let terminal = ticket.terminal_id().ok_or_else(|| backend_fault(false))?;
                let stopped = self.stop_all(true);
                if self.active.is_empty() {
                    self.applied_terminal = Some(terminal);
                }
                stopped?;
                Ok(ticket.acknowledge(true))
            }
        }
    }

    fn poll(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        self.require_binding(ticket.binding())?;
        if let Some(event) = self.poll_lifecycle()? {
            return Ok(Some(event));
        }
        if let Some(event) = self.poll_catalog()? {
            return Ok(Some(event));
        }
        if let Some(event) = self.poll_permission()? {
            return Ok(Some(event));
        }
        self.poll_round_robin()
    }
}

#[cfg(target_os = "macos")]
pub struct MacOsDeviceAvBridge {
    inner: DeviceInputBridge<MacOsDeviceInputs, NativePowerEvents>,
}

#[cfg(target_os = "macos")]
impl MacOsDeviceAvBridge {
    pub fn new(
        installation_secret: [u8; 32],
        power: &SystemPowerMonitor,
    ) -> Result<Self, MacOsDeviceAvBridgeCreateError> {
        let adapter = derive_opaque_id(&installation_secret, ADAPTER_ID_DOMAIN)
            .and_then(|bytes| AvAdapterInstanceId::from_opaque(bytes).ok())
            .ok_or(MacOsDeviceAvBridgeCreateError::AdapterIdentity)?;
        let inputs = MacOsDeviceInputs::new(installation_secret)?;
        let inner = DeviceInputBridge::new(inputs, NativePowerEvents(power.cursor()), adapter)
            .map_err(|_| MacOsDeviceAvBridgeCreateError::DeviceMonitor)?;
        Ok(Self { inner })
    }
}

#[cfg(target_os = "macos")]
impl NativeAvBridge for MacOsDeviceAvBridge {
    fn adapter_instance(&self) -> AvAdapterInstanceId {
        self.inner.adapter_instance()
    }

    fn bind(&mut self, ticket: AvSessionClaimTicket) -> Result<AvOwnerBinding, NativeAvFailure> {
        self.inner.bind(ticket)
    }

    fn capabilities(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<NativeAvBridgeCapabilities, NativeAvFailure> {
        self.inner.capabilities(ticket)
    }

    fn enumerate(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<AvDeviceCatalog, NativeAvFailure> {
        self.inner.enumerate(ticket)
    }

    fn startup_calibration(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        self.inner.startup_calibration(ticket, stamp)
    }

    fn reconcile_terminal(
        &mut self,
        ticket: AvTerminalReconcileTicket,
    ) -> Result<AvTerminalPostcondition, NativeAvFailure> {
        self.inner.reconcile_terminal(ticket)
    }

    fn execute(
        &mut self,
        ticket: AvOperationTicket,
        request: &AvNativeRequest,
    ) -> Result<NativeAvAcknowledgement, NativeAvFailure> {
        self.inner.execute(ticket, request)
    }

    fn poll(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        self.inner.poll(ticket)
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct DeviceRecord {
    descriptor: InputDevice,
    device: gst::Device,
}

#[cfg(target_os = "macos")]
struct MacOsDeviceInputs {
    installation_secret: [u8; 32],
    monitor: gst::DeviceMonitor,
    records: Vec<DeviceRecord>,
    permissions: BTreeMap<AvSourceClass, PermissionState>,
    pipelines: BTreeMap<AvSourceClass, CapturePipeline>,
}

#[cfg(target_os = "macos")]
impl MacOsDeviceInputs {
    fn new(installation_secret: [u8; 32]) -> Result<Self, MacOsDeviceAvBridgeCreateError> {
        if installation_secret.iter().all(|byte| *byte == 0) {
            return Err(MacOsDeviceAvBridgeCreateError::AdapterIdentity);
        }
        prepare_runtime().map_err(|_| MacOsDeviceAvBridgeCreateError::Runtime)?;
        require_capture_factories()?;
        let monitor = gst::DeviceMonitor::new();
        if monitor.add_filter(Some("Audio/Source"), None).is_none()
            || monitor.add_filter(Some("Video/Source"), None).is_none()
            || monitor.start().is_err()
        {
            return Err(MacOsDeviceAvBridgeCreateError::DeviceMonitor);
        }
        let mut inputs = Self {
            installation_secret,
            monitor,
            records: Vec::new(),
            permissions: BTreeMap::from([
                (AvSourceClass::Microphone, PermissionState::PromptRequired),
                (AvSourceClass::Camera, PermissionState::PromptRequired),
            ]),
            pipelines: BTreeMap::new(),
        };
        inputs
            .refresh_inventory()
            .map_err(|_| MacOsDeviceAvBridgeCreateError::DeviceMonitor)?;
        while inputs.monitor.bus().pop().is_some() {}
        Ok(inputs)
    }

    fn refresh_inventory(&mut self) -> Result<(), InputFailure> {
        let mut records = Vec::new();
        for device in self.monitor.devices() {
            let class = if device.has_classes("Audio/Source") {
                AvSourceClass::Microphone
            } else if device.has_classes("Video/Source") {
                AvSourceClass::Camera
            } else {
                continue;
            };
            let identity = device_identity(&device);
            let domain = match class {
                AvSourceClass::Microphone => MICROPHONE_DEVICE_ID_DOMAIN,
                AvSourceClass::Camera => CAMERA_DEVICE_ID_DOMAIN,
                AvSourceClass::SystemAudio => continue,
            };
            let id = derive_device_id(&self.installation_secret, domain, identity.as_bytes())?;
            let is_default = device
                .properties()
                .and_then(|properties| properties.get::<bool>("is-default").ok())
                .unwrap_or(false);
            records.push(DeviceRecord {
                descriptor: InputDevice {
                    id,
                    generation: AvDeviceGeneration::new(1)
                        .map_err(|_| InputFailure::BackendFault)?,
                    class,
                    is_default,
                    permission: PermissionState::Granted,
                    route: device_route(&device),
                    formats: vec![normalized_format(class)],
                },
                device,
            });
        }
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            let mut matching = records
                .iter_mut()
                .filter(|record| record.descriptor.class == class)
                .peekable();
            if matching.peek().is_some() {
                self.permissions.insert(class, PermissionState::Granted);
                if !matching.any(|record| record.descriptor.is_default)
                    && let Some(first) = records
                        .iter_mut()
                        .find(|record| record.descriptor.class == class)
                {
                    first.descriptor.is_default = true;
                }
            }
        }
        records.sort_by_key(|record| (record.descriptor.class, record.descriptor.id));
        self.records = records;
        Ok(())
    }

    fn fallback_device(&self, class: AvSourceClass) -> Result<InputDevice, InputFailure> {
        let domain = match class {
            AvSourceClass::Microphone => MICROPHONE_DEVICE_ID_DOMAIN,
            AvSourceClass::Camera => CAMERA_DEVICE_ID_DOMAIN,
            AvSourceClass::SystemAudio => return Err(InputFailure::SourceUnavailable),
        };
        let id = derive_device_id(&self.installation_secret, domain, b"default")?;
        Ok(InputDevice {
            id,
            generation: AvDeviceGeneration::new(1).map_err(|_| InputFailure::BackendFault)?,
            class,
            is_default: true,
            permission: self
                .permissions
                .get(&class)
                .copied()
                .unwrap_or(PermissionState::PromptRequired),
            route: NativeRouteClass::Unknown,
            formats: vec![normalized_format(class)],
        })
    }

    fn pipeline_source(
        &self,
        class: AvSourceClass,
        device: AvDeviceId,
    ) -> Result<gst::Element, InputFailure> {
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.descriptor.class == class && record.descriptor.id == device)
        {
            return record
                .device
                .create_element(Some("native_source"))
                .map_err(|_| InputFailure::SourceUnavailable);
        }
        let fallback = self.fallback_device(class)?;
        if fallback.id != device {
            return Err(InputFailure::SourceUnavailable);
        }
        source_factory(class)
    }
}

#[cfg(target_os = "macos")]
impl DeviceInputs for MacOsDeviceInputs {
    fn devices(&mut self) -> Result<Vec<InputDevice>, InputFailure> {
        let mut devices = self
            .records
            .iter()
            .map(|record| record.descriptor.clone())
            .collect::<Vec<_>>();
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            if !devices.iter().any(|device| device.class == class) {
                devices.push(self.fallback_device(class)?);
            }
        }
        devices.sort_by_key(|device| (device.class, device.id));
        Ok(devices)
    }

    fn permission(&mut self, class: AvSourceClass) -> Result<PermissionState, InputFailure> {
        self.permissions
            .get(&class)
            .copied()
            .ok_or(InputFailure::SourceUnavailable)
    }

    fn request_permission(
        &mut self,
        class: AvSourceClass,
        timeout: Duration,
    ) -> Result<PermissionState, InputFailure> {
        if !matches!(class, AvSourceClass::Microphone | AvSourceClass::Camera) {
            return Err(InputFailure::SourceUnavailable);
        }
        let fallback = self.fallback_device(class)?;
        let source = source_factory(class)?;
        let mut pipeline =
            CapturePipeline::build(source, normalized_format(class), Instant::now())?;
        pipeline.start()?;
        let timeout = gst::ClockTime::try_from(timeout)
            .unwrap_or(PERMISSION_SAMPLE_TIMEOUT)
            .min(PERMISSION_SAMPLE_TIMEOUT);
        let result = pipeline.pull_sample(timeout);
        let stopped = pipeline.stop_and_drain();
        result?;
        stopped.map_err(|error| error.failure)?;
        self.permissions.insert(class, PermissionState::Granted);
        self.refresh_inventory()?;
        if self
            .records
            .iter()
            .any(|record| record.descriptor.class == class)
            || fallback.id
                == self
                    .fallback_device(class)
                    .map(|device| device.id)
                    .unwrap_or(fallback.id)
        {
            Ok(PermissionState::Granted)
        } else {
            Err(InputFailure::SourceUnavailable)
        }
    }

    fn start(
        &mut self,
        class: AvSourceClass,
        device: AvDeviceId,
        format: AvFormat,
        master_origin: Instant,
    ) -> Result<(), InputFailure> {
        if self.pipelines.contains_key(&class) {
            return Err(InputFailure::Busy);
        }
        if self.permission(class)? != PermissionState::Granted {
            return Err(InputFailure::PermissionDenied);
        }
        let source = self.pipeline_source(class, device)?;
        let mut pipeline = CapturePipeline::build(source, format, master_origin)?;
        pipeline.start()?;
        self.pipelines.insert(class, pipeline);
        Ok(())
    }

    fn poll(&mut self, class: AvSourceClass) -> Result<Option<InputBuffer>, InputFailure> {
        self.pipelines
            .get_mut(&class)
            .ok_or(InputFailure::SourceUnavailable)?
            .try_pull()
    }

    fn stop_and_drain(
        &mut self,
        class: AvSourceClass,
    ) -> Result<Vec<InputBuffer>, InputStopFailure> {
        let Some(mut pipeline) = self.pipelines.remove(&class) else {
            return Ok(Vec::new());
        };
        pipeline.stop_and_drain()
    }

    fn is_running(&self, class: AvSourceClass) -> bool {
        self.pipelines.contains_key(&class)
    }

    fn poll_catalog_change(&mut self) -> Result<Option<CatalogChangeReason>, InputFailure> {
        let mut changed = false;
        let bus = self.monitor.bus();
        while let Some(message) = bus.pop() {
            if matches!(
                message.view(),
                gst::MessageView::DeviceAdded(_) | gst::MessageView::DeviceRemoved(_)
            ) {
                changed = true;
            }
        }
        if !changed {
            return Ok(None);
        }
        let before = self
            .records
            .iter()
            .map(|record| {
                (
                    record.descriptor.class,
                    record.descriptor.id,
                    record.descriptor.is_default,
                )
            })
            .collect::<Vec<_>>();
        self.refresh_inventory()?;
        let after = self
            .records
            .iter()
            .map(|record| {
                (
                    record.descriptor.class,
                    record.descriptor.id,
                    record.descriptor.is_default,
                )
            })
            .collect::<Vec<_>>();
        if before == after {
            Ok(None)
        } else if before
            .iter()
            .map(|(class, id, _)| (*class, *id))
            .eq(after.iter().map(|(class, id, _)| (*class, *id)))
        {
            Ok(Some(CatalogChangeReason::DefaultChanged))
        } else {
            Ok(Some(CatalogChangeReason::Hotplug))
        }
    }
}

#[cfg(target_os = "macos")]
struct CapturePipeline {
    pipeline: gst::Pipeline,
    sink: gst_app::AppSink,
    format: AvFormat,
    master_origin: Instant,
    running: bool,
}

#[cfg(target_os = "macos")]
impl CapturePipeline {
    fn build(
        source: gst::Element,
        format: AvFormat,
        master_origin: Instant,
    ) -> Result<Self, InputFailure> {
        let pipeline = gst::Pipeline::with_name("frame_native_device_input");
        let max_bytes =
            u32::try_from(max_input_bytes(format)?).map_err(|_| InputFailure::FormatChanged)?;
        let queue = gst::ElementFactory::make("queue")
            .property("max-size-buffers", SOURCE_QUEUE_BUFFERS)
            .property("max-size-bytes", max_bytes)
            .property("max-size-time", 500_000_000_u64)
            .property_from_str("leaky", "downstream")
            .build()
            .map_err(|_| InputFailure::SourceUnavailable)?;
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .property("caps", capture_caps(format)?)
            .build()
            .map_err(|_| InputFailure::SourceUnavailable)?;
        let sink = gst::ElementFactory::make("appsink")
            .name("native_sink")
            .property("sync", false)
            .property("async", false)
            .property("wait-on-eos", false)
            .property("enable-last-sample", false)
            .property("max-buffers", SOURCE_QUEUE_BUFFERS)
            .property("drop", true)
            .build()
            .map_err(|_| InputFailure::SourceUnavailable)?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| InputFailure::BackendFault)?;
        match format {
            AvFormat::Audio(_) => {
                let convert = gst::ElementFactory::make("audioconvert")
                    .build()
                    .map_err(|_| InputFailure::SourceUnavailable)?;
                let resample = gst::ElementFactory::make("audioresample")
                    .build()
                    .map_err(|_| InputFailure::SourceUnavailable)?;
                pipeline
                    .add_many([
                        &source,
                        &convert,
                        &resample,
                        &capsfilter,
                        queue.upcast_ref(),
                        sink.upcast_ref(),
                    ])
                    .map_err(|_| InputFailure::BackendFault)?;
                gst::Element::link_many([
                    &source,
                    &convert,
                    &resample,
                    &capsfilter,
                    queue.upcast_ref(),
                    sink.upcast_ref(),
                ])
                .map_err(|_| InputFailure::BackendFault)?;
            }
            AvFormat::Camera(_) => {
                let convert = gst::ElementFactory::make("videoconvert")
                    .build()
                    .map_err(|_| InputFailure::SourceUnavailable)?;
                let scale = gst::ElementFactory::make("videoscale")
                    .build()
                    .map_err(|_| InputFailure::SourceUnavailable)?;
                pipeline
                    .add_many([
                        &source,
                        &convert,
                        &scale,
                        &capsfilter,
                        queue.upcast_ref(),
                        sink.upcast_ref(),
                    ])
                    .map_err(|_| InputFailure::BackendFault)?;
                gst::Element::link_many([
                    &source,
                    &convert,
                    &scale,
                    &capsfilter,
                    queue.upcast_ref(),
                    sink.upcast_ref(),
                ])
                .map_err(|_| InputFailure::BackendFault)?;
            }
        }
        if !pipeline_has_only_declared_authored_factories(&pipeline)
            || !pipeline_has_trusted_factory_provenance(&pipeline)
        {
            return Err(InputFailure::BackendFault);
        }
        Ok(Self {
            pipeline,
            sink,
            format,
            master_origin,
            running: false,
        })
    }

    fn start(&mut self) -> Result<(), InputFailure> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| InputFailure::SourceUnavailable)?;
        let (transition, current, _) = self.pipeline.state(SOURCE_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Playing {
            let _ = self.pipeline.set_state(gst::State::Null);
            return Err(InputFailure::SourceUnavailable);
        }
        if !pipeline_has_trusted_factory_provenance(&self.pipeline) {
            let _ = self.pipeline.set_state(gst::State::Null);
            return Err(InputFailure::BackendFault);
        }
        self.running = true;
        Ok(())
    }

    fn try_pull(&mut self) -> Result<Option<InputBuffer>, InputFailure> {
        self.poll_bus()?;
        self.sink
            .try_pull_sample(gst::ClockTime::ZERO)
            .map(|sample| self.convert_sample(sample))
            .transpose()
    }

    fn pull_sample(&mut self, timeout: gst::ClockTime) -> Result<InputBuffer, InputFailure> {
        self.poll_bus()?;
        self.sink
            .try_pull_sample(timeout)
            .ok_or(InputFailure::Timeout)
            .and_then(|sample| self.convert_sample(sample))
    }

    fn convert_sample(&self, sample: gst::Sample) -> Result<InputBuffer, InputFailure> {
        let buffer = sample.buffer().ok_or(InputFailure::BackendFault)?;
        let pts = buffer.pts().ok_or(InputFailure::FormatChanged)?.nseconds();
        let duration_ns = buffer
            .duration()
            .map(gst::ClockTime::nseconds)
            .unwrap_or_else(|| nominal_duration_ns(self.format));
        if duration_ns == 0 {
            return Err(InputFailure::FormatChanged);
        }
        let map = buffer
            .map_readable()
            .map_err(|_| InputFailure::BackendFault)?;
        let bytes = map.as_slice();
        if bytes.is_empty()
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_input_bytes(self.format)?
        {
            return Err(InputFailure::FormatChanged);
        }
        validate_payload_shape(self.format, bytes.len())?;
        Ok(InputBuffer {
            format: self.format,
            source_pts_ns: pts,
            arrival_ns: u64::try_from(self.master_origin.elapsed().as_nanos()).unwrap_or(u64::MAX),
            duration_ns,
            discontinuity: buffer.flags().contains(gst::BufferFlags::DISCONT),
            bytes: bytes.to_vec(),
        })
    }

    fn poll_bus(&self) -> Result<(), InputFailure> {
        let Some(bus) = self.pipeline.bus() else {
            return Err(InputFailure::BackendFault);
        };
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Error(_) | gst::MessageView::Eos(_) => {
                    return Err(InputFailure::SourceUnavailable);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn stop_and_drain(&mut self) -> Result<Vec<InputBuffer>, InputStopFailure> {
        if !self.running {
            return Ok(Vec::new());
        }
        if self.pipeline.set_state(gst::State::Paused).is_err() {
            return Err(InputStopFailure {
                failure: InputFailure::BackendFault,
                teardown_confirmed: false,
            });
        }
        let mut tail = Vec::new();
        while tail.len() < SOURCE_QUEUE_BUFFERS as usize {
            match self.sink.try_pull_sample(gst::ClockTime::ZERO) {
                Some(sample) => match self.convert_sample(sample) {
                    Ok(input) => tail.push(input),
                    Err(failure) => {
                        let _ = self.pipeline.set_state(gst::State::Null);
                        self.running = false;
                        return Err(InputStopFailure {
                            failure,
                            teardown_confirmed: true,
                        });
                    }
                },
                None => break,
            }
        }
        if self.pipeline.set_state(gst::State::Null).is_err() {
            return Err(InputStopFailure {
                failure: InputFailure::BackendFault,
                teardown_confirmed: false,
            });
        }
        let (transition, current, _) = self.pipeline.state(SOURCE_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Null {
            return Err(InputStopFailure {
                failure: InputFailure::Timeout,
                teardown_confirmed: false,
            });
        }
        self.running = false;
        Ok(tail)
    }
}

#[cfg(target_os = "macos")]
impl Drop for CapturePipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(target_os = "macos")]
fn require_capture_factories() -> Result<(), MacOsDeviceAvBridgeCreateError> {
    for (factory, capability) in [
        ("osxaudiosrc", RuntimeCapability::MicrophoneCapture),
        ("avfvideosrc", RuntimeCapability::CameraCapture),
    ] {
        let declared = runtime_manifest().factories.iter().any(|spec| {
            spec.factory == factory
                && spec.capability == capability
                && spec.requirement == FactoryRequirement::Optional
                && spec.platform == PlatformScope::MacOs
        });
        if !declared {
            return Err(MacOsDeviceAvBridgeCreateError::Runtime);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn source_factory(class: AvSourceClass) -> Result<gst::Element, InputFailure> {
    let factory = match class {
        AvSourceClass::Microphone => "osxaudiosrc",
        AvSourceClass::Camera => "avfvideosrc",
        AvSourceClass::SystemAudio => return Err(InputFailure::SourceUnavailable),
    };
    gst::ElementFactory::make(factory)
        .name("native_source")
        .build()
        .map_err(|_| InputFailure::SourceUnavailable)
}

#[cfg(target_os = "macos")]
fn capture_caps(format: AvFormat) -> Result<gst::Caps, InputFailure> {
    match format {
        AvFormat::Audio(format) if format == MICROPHONE_FORMAT => {
            Ok(gst::Caps::builder("audio/x-raw")
                .field("format", "F32LE")
                .field("layout", "interleaved")
                .field(
                    "rate",
                    i32::try_from(format.sample_rate).map_err(|_| InputFailure::FormatChanged)?,
                )
                .field("channels", i32::from(format.channels))
                .build())
        }
        AvFormat::Camera(format) if format == CAMERA_FORMAT => {
            Ok(gst::Caps::builder("video/x-raw")
                .field("format", "BGRA")
                .field(
                    "width",
                    i32::try_from(format.width).map_err(|_| InputFailure::FormatChanged)?,
                )
                .field(
                    "height",
                    i32::try_from(format.height).map_err(|_| InputFailure::FormatChanged)?,
                )
                .field(
                    "framerate",
                    gst::Fraction::new(
                        i32::try_from(format.frame_rate_numerator)
                            .map_err(|_| InputFailure::FormatChanged)?,
                        i32::try_from(format.frame_rate_denominator)
                            .map_err(|_| InputFailure::FormatChanged)?,
                    ),
                )
                .build())
        }
        _ => Err(InputFailure::FormatChanged),
    }
}

#[cfg(target_os = "macos")]
const fn normalized_format(class: AvSourceClass) -> AvFormat {
    match class {
        AvSourceClass::Microphone => AvFormat::Audio(MICROPHONE_FORMAT),
        AvSourceClass::Camera => AvFormat::Camera(CAMERA_FORMAT),
        AvSourceClass::SystemAudio => AvFormat::Audio(SYSTEM_AUDIO_FORMAT),
    }
}

#[cfg(target_os = "macos")]
fn nominal_duration_ns(format: AvFormat) -> u64 {
    match format {
        AvFormat::Audio(_) => 10_000_000,
        AvFormat::Camera(format) => {
            u64::from(format.frame_rate_denominator).saturating_mul(1_000_000_000)
                / u64::from(format.frame_rate_numerator.max(1))
        }
    }
}

#[cfg(target_os = "macos")]
fn max_input_bytes(format: AvFormat) -> Result<u64, InputFailure> {
    match format {
        AvFormat::Audio(format) => u64::from(format.sample_rate)
            .checked_mul(u64::from(format.channels))
            .and_then(|bytes| bytes.checked_mul(4))
            .map(|bytes| bytes / 10)
            .ok_or(InputFailure::FormatChanged),
        AvFormat::Camera(format) => u64::from(format.width)
            .checked_mul(u64::from(format.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(InputFailure::FormatChanged),
    }
}

#[cfg(target_os = "macos")]
fn validate_payload_shape(format: AvFormat, bytes: usize) -> Result<(), InputFailure> {
    match format {
        AvFormat::Audio(format) => {
            let frame_bytes = usize::from(format.channels)
                .checked_mul(4)
                .ok_or(InputFailure::FormatChanged)?;
            if !bytes.is_multiple_of(frame_bytes) {
                return Err(InputFailure::FormatChanged);
            }
        }
        AvFormat::Camera(format) => {
            let expected = usize::try_from(
                u64::from(format.width)
                    .checked_mul(u64::from(format.height))
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or(InputFailure::FormatChanged)?,
            )
            .map_err(|_| InputFailure::FormatChanged)?;
            if bytes != expected {
                return Err(InputFailure::FormatChanged);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn device_identity(device: &gst::Device) -> String {
    let properties = device.properties();
    for key in [
        "unique-id",
        "device.id",
        "device.path",
        "device-uid",
        "device-index",
    ] {
        if let Some(value) = properties
            .as_ref()
            .and_then(|properties| properties.get::<String>(key).ok())
            .filter(|value| !value.is_empty() && value.len() <= 4096)
        {
            return value;
        }
    }
    device.display_name().as_str().chars().take(4096).collect()
}

#[cfg(target_os = "macos")]
fn device_route(device: &gst::Device) -> NativeRouteClass {
    let transport = device.properties().and_then(|properties| {
        ["transport", "device.transport", "device.api"]
            .into_iter()
            .find_map(|key| properties.get::<String>(key).ok())
    });
    match transport.as_deref().map(str::to_ascii_lowercase) {
        Some(value) if value.contains("bluetooth") => NativeRouteClass::WirelessWideband,
        Some(value) if value.contains("usb") => NativeRouteClass::Wired,
        Some(value) if value.contains("built") || value.contains("coreaudio") => {
            NativeRouteClass::BuiltIn
        }
        _ => NativeRouteClass::Unknown,
    }
}

#[cfg(target_os = "macos")]
fn derive_device_id(
    installation_secret: &[u8; 32],
    domain: &[u8],
    provider_identity: &[u8],
) -> Result<AvDeviceId, InputFailure> {
    if provider_identity.is_empty() || provider_identity.len() > 4096 {
        return Err(InputFailure::BackendFault);
    }
    let mut message = Vec::with_capacity(domain.len() + provider_identity.len());
    message.extend_from_slice(domain);
    message.extend_from_slice(provider_identity);
    let opaque =
        derive_opaque_id(installation_secret, &message).ok_or(InputFailure::BackendFault)?;
    AvDeviceId::from_opaque(opaque).map_err(|_| InputFailure::BackendFault)
}

#[cfg(target_os = "macos")]
fn derive_opaque_id(installation_secret: &[u8; 32], message: &[u8]) -> Option<[u8; 16]> {
    if installation_secret.iter().all(|byte| *byte == 0) || message.is_empty() {
        return None;
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, installation_secret);
    let digest = hmac::sign(&key, message);
    let mut opaque = [0_u8; 16];
    opaque.copy_from_slice(&digest.as_ref()[..16]);
    Some(opaque)
}

fn contract_failure(_error: AvCaptureError) -> NativeAvFailure {
    backend_fault(false)
}

const fn backend_fault(retryable: bool) -> NativeAvFailure {
    NativeAvFailure {
        code: NativeAvFailureCode::BackendFault,
        retryable,
    }
}

const fn capability_changed() -> NativeAvFailure {
    NativeAvFailure {
        code: NativeAvFailureCode::CapabilityChanged,
        retryable: true,
    }
}

const fn timeout_failure() -> NativeAvFailure {
    NativeAvFailure {
        code: NativeAvFailureCode::Timeout,
        retryable: true,
    }
}

const fn map_input_failure(error: InputFailure) -> NativeAvFailure {
    let (code, retryable) = match error {
        InputFailure::PermissionDenied => (NativeAvFailureCode::PermissionDenied, false),
        InputFailure::SourceUnavailable => (NativeAvFailureCode::SourceUnavailable, true),
        InputFailure::Busy => (NativeAvFailureCode::Busy, true),
        InputFailure::Timeout => (NativeAvFailureCode::Timeout, true),
        InputFailure::FormatChanged => (NativeAvFailureCode::FormatChanged, true),
        InputFailure::BackendFault => (NativeAvFailureCode::BackendFault, false),
    };
    NativeAvFailure { code, retryable }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use frame_media::{
        AV_SETTINGS_VERSION, AvActionExecution, AvCaptureSession, AvCaptureSettingsV2,
        AvRuntimePolicy, AvSessionId, AvSyncPolicy, BoundNativeAvBridge, DeviceSelectionV2,
        NativeAvGstreamerGraph, NativeAvRuntime, NativeAvRuntimeOutcome, NativeAvSourceTeardown,
        prepare_runtime,
    };

    use super::*;

    #[derive(Default)]
    struct FakePower {
        events: VecDeque<SystemPowerEvent>,
    }

    impl PowerEvents for FakePower {
        fn poll_power(&mut self) -> Result<Option<SystemPowerEvent>, SystemPowerMonitorError> {
            Ok(self.events.pop_front())
        }
    }

    struct FakeInputState {
        devices: Vec<InputDevice>,
        running: BTreeMap<AvSourceClass, bool>,
        buffers: BTreeMap<AvSourceClass, VecDeque<InputBuffer>>,
        catalog_changes: VecDeque<CatalogChangeReason>,
        stops: u8,
    }

    struct FakeInputs {
        state: Arc<Mutex<FakeInputState>>,
    }

    impl FakeInputs {
        fn new() -> (Self, Arc<Mutex<FakeInputState>>) {
            let devices = vec![
                InputDevice {
                    id: device_id(3),
                    generation: generation(),
                    class: AvSourceClass::Microphone,
                    is_default: true,
                    permission: PermissionState::Granted,
                    route: NativeRouteClass::BuiltIn,
                    formats: vec![AvFormat::Audio(MICROPHONE_FORMAT)],
                },
                InputDevice {
                    id: device_id(4),
                    generation: generation(),
                    class: AvSourceClass::Camera,
                    is_default: true,
                    permission: PermissionState::Granted,
                    route: NativeRouteClass::BuiltIn,
                    formats: vec![AvFormat::Camera(CAMERA_FORMAT)],
                },
            ];
            let state = Arc::new(Mutex::new(FakeInputState {
                devices,
                running: BTreeMap::new(),
                buffers: BTreeMap::new(),
                catalog_changes: VecDeque::new(),
                stops: 0,
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    impl DeviceInputs for FakeInputs {
        fn devices(&mut self) -> Result<Vec<InputDevice>, InputFailure> {
            Ok(self.state.lock().expect("state").devices.clone())
        }

        fn permission(&mut self, class: AvSourceClass) -> Result<PermissionState, InputFailure> {
            self.state
                .lock()
                .expect("state")
                .devices
                .iter()
                .find(|device| device.class == class)
                .map(|device| device.permission)
                .ok_or(InputFailure::SourceUnavailable)
        }

        fn request_permission(
            &mut self,
            class: AvSourceClass,
            _timeout: Duration,
        ) -> Result<PermissionState, InputFailure> {
            self.permission(class)
        }

        fn start(
            &mut self,
            class: AvSourceClass,
            device: AvDeviceId,
            format: AvFormat,
            _master_origin: Instant,
        ) -> Result<(), InputFailure> {
            let mut state = self.state.lock().expect("state");
            if !state.devices.iter().any(|candidate| {
                candidate.class == class
                    && candidate.id == device
                    && candidate.formats.contains(&format)
            }) {
                return Err(InputFailure::SourceUnavailable);
            }
            state.running.insert(class, true);
            Ok(())
        }

        fn poll(&mut self, class: AvSourceClass) -> Result<Option<InputBuffer>, InputFailure> {
            let mut state = self.state.lock().expect("state");
            if state.running.get(&class).copied() != Some(true) {
                return Err(InputFailure::SourceUnavailable);
            }
            Ok(state.buffers.entry(class).or_default().pop_front())
        }

        fn stop_and_drain(
            &mut self,
            class: AvSourceClass,
        ) -> Result<Vec<InputBuffer>, InputStopFailure> {
            let mut state = self.state.lock().expect("state");
            state.running.insert(class, false);
            state.stops = state.stops.saturating_add(1);
            Ok(state.buffers.entry(class).or_default().drain(..).collect())
        }

        fn is_running(&self, class: AvSourceClass) -> bool {
            self.state
                .lock()
                .expect("state")
                .running
                .get(&class)
                .copied()
                == Some(true)
        }

        fn poll_catalog_change(&mut self) -> Result<Option<CatalogChangeReason>, InputFailure> {
            Ok(self
                .state
                .lock()
                .expect("state")
                .catalog_changes
                .pop_front())
        }
    }

    fn adapter() -> AvAdapterInstanceId {
        AvAdapterInstanceId::from_opaque([7; 16]).expect("adapter")
    }

    fn session_id(seed: u8) -> AvSessionId {
        AvSessionId::from_csprng([seed; 16]).expect("session")
    }

    fn device_id(seed: u8) -> AvDeviceId {
        AvDeviceId::from_opaque([seed; 16]).expect("device")
    }

    fn generation() -> AvDeviceGeneration {
        AvDeviceGeneration::new(1).expect("generation")
    }

    fn settings() -> AvCaptureSettingsV2 {
        AvCaptureSettingsV2 {
            version: AV_SETTINGS_VERSION,
            microphone: DeviceSelectionV2::Pinned {
                id: device_id(3),
                format: AvFormat::Audio(MICROPHONE_FORMAT),
            },
            system_audio: DeviceSelectionV2::Disabled,
            camera: DeviceSelectionV2::Pinned {
                id: device_id(4),
                format: AvFormat::Camera(CAMERA_FORMAT),
            },
        }
    }

    fn input(class: AvSourceClass, sequence: u64) -> InputBuffer {
        let (format, bytes, duration_ns) = match class {
            AvSourceClass::Microphone => (
                AvFormat::Audio(MICROPHONE_FORMAT),
                vec![0_u8; 3_840],
                10_000_000,
            ),
            AvSourceClass::Camera => (
                AvFormat::Camera(CAMERA_FORMAT),
                vec![0_u8; 1280 * 720 * 4],
                33_333_333,
            ),
            AvSourceClass::SystemAudio => unreachable!(),
        };
        InputBuffer {
            format,
            source_pts_ns: 9_000_000_000 + sequence * duration_ns,
            arrival_ns: 5_000_000 + sequence * duration_ns,
            duration_ns,
            discontinuity: sequence == 0,
            bytes,
        }
    }

    fn enqueue_calibration(state: &Arc<Mutex<FakeInputState>>) {
        let mut state = state.lock().expect("state");
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            let queue = state.buffers.entry(class).or_default();
            for sequence in 0..7 {
                queue.push_back(input(class, sequence));
            }
        }
    }

    type StartedBridge = (
        BoundNativeAvBridge<DeviceInputBridge<FakeInputs, FakePower>>,
        AvCaptureSession,
        AvPipelineGraphSpec,
        Arc<Mutex<FakeInputState>>,
    );

    fn started(seed: u8) -> StartedBridge {
        let (inputs, state) = FakeInputs::new();
        enqueue_calibration(&state);
        let bridge =
            DeviceInputBridge::new(inputs, FakePower::default(), adapter()).expect("bridge");
        let mut bound = BoundNativeAvBridge::new(bridge, session_id(seed)).expect("bound");
        let mut session = AvCaptureSession::new(bound.claim_session().expect("owner"));
        let capabilities = bound.capabilities().expect("capabilities");
        let catalog = bound.enumerate().expect("catalog");
        let graph = AvPipelineGraphSpec::negotiate(&catalog, settings(), true).expect("graph");
        let action = session
            .request_start(capabilities, catalog, settings(), true)
            .expect("start");
        let AvActionExecution::Acknowledged(ack) = action
            .execute_source(&mut session, &mut bound)
            .expect("execute")
        else {
            panic!("start must acknowledge");
        };
        session.complete(ack).expect("complete");
        (bound, session, graph, state)
    }

    #[test]
    fn bridge_calibrates_and_delivers_owned_microphone_and_camera_buffers() {
        let (mut bridge, mut session, _, _) = started(1);
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            let stamp = session.source_stamp(class).expect("stamp");
            let batch = bridge
                .startup_calibration(stamp)
                .expect("calibration batch");
            assert_eq!(batch.samples().len(), STARTUP_CALIBRATION_SAMPLES);
            session
                .calibrate_source(stamp, AvSyncPolicy::default(), batch.samples())
                .expect("install calibration");
        }
        for _ in 0..4 {
            session.poll_source(&mut bridge).expect("poll");
        }
        for class in [AvSourceClass::Microphone, AvSourceClass::Camera] {
            let input = session
                .pop_buffer(class, MonotonicTimeNs::new(250_000_000))
                .expect("pop")
                .expect("owned input")
                .into_appsrc_input()
                .expect("appsrc input");
            assert!(!input.payload().bytes().expect("bytes").is_empty());
            input.release();
        }
    }

    #[test]
    fn production_appsrc_graph_consumes_both_device_sources_and_tears_down_once() {
        prepare_runtime().expect("GStreamer runtime");
        let (bridge, session, graph_spec, state) = started(2);
        let graph = NativeAvGstreamerGraph::build(&graph_spec).expect("native graph");
        let mut runtime = NativeAvRuntime::attach(
            bridge,
            session,
            graph,
            AvSyncPolicy::default(),
            AvRuntimePolicy::default(),
        )
        .expect("runtime");
        let report = runtime
            .poll(MonotonicTimeNs::new(300_000_000))
            .expect("poll runtime");
        assert!(report.buffers_pushed >= 2, "report: {report:?}");
        let termination = runtime.cancel().expect("cancel");
        assert_eq!(termination.outcome, NativeAvRuntimeOutcome::Cancelled);
        assert_eq!(
            termination.source_teardown,
            NativeAvSourceTeardown::Confirmed
        );
        assert_eq!(state.lock().expect("state").stops, 2);
    }

    #[test]
    fn device_change_is_owner_stamped_and_never_exposes_provider_identity() {
        let (mut bridge, mut session, _, state) = started(3);
        state
            .lock()
            .expect("state")
            .catalog_changes
            .push_back(CatalogChangeReason::Hotplug);
        state
            .lock()
            .expect("state")
            .devices
            .retain(|device| device.class != AvSourceClass::Camera);
        let outcome = session
            .poll_source(&mut bridge)
            .expect("poll")
            .expect("catalog event");
        assert!(outcome.native_reconfigure_required);
        let debug = format!("{outcome:?}");
        assert!(!debug.contains("Built-in"));
        assert!(!debug.contains("provider"));
    }
}
