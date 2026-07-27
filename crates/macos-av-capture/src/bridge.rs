//! Provider-neutral bridge for the bounded macOS system-audio primitive.
//!
//! The production wrapper is macOS-only. The generic core remains portable in
//! tests so ownership, calibration, lifecycle, and teardown behavior do not
//! depend on CI having physical audio hardware.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use frame_media::{
    AV_CAPTURE_CONTRACT_VERSION, AvAdapterInstanceId, AvBufferLease, AvCaptureError,
    AvControlEventStamp, AvDeviceCatalog, AvDeviceDescriptor, AvDeviceGeneration, AvFormat,
    AvNativeRequest, AvOperationTicket, AvOwnerBinding, AvPayloadBody, AvPipelineGraphSpec,
    AvSessionClaimTicket, AvSourceCallTicket, AvSourceClass, AvSourceStamp,
    AvTerminalPostcondition, AvTerminalReconcileTicket, CalibrationSample, LatencyConfidence,
    MonotonicTimeNs, NativeAvAcknowledgement, NativeAvBridge, NativeAvBridgeCapabilities,
    NativeAvBuffer, NativeAvBufferTiming, NativeAvCalibrationBatch, NativeAvEvent, NativeAvFailure,
    NativeAvFailureCode, NativeAvTerminalBuffer, NativeRouteClass, NativeTimestampKind,
    PermissionPreflight, PermissionState, SourceLatency,
};
#[cfg(target_os = "macos")]
use frame_platform_lifecycle::{SystemPowerCursor, SystemPowerMonitor};
use frame_platform_lifecycle::{SystemPowerEvent, SystemPowerMonitorError};
#[cfg(target_os = "macos")]
use ring::hmac;
#[cfg(target_os = "macos")]
use thiserror::Error;

use crate::{
    MacOsSystemAudioChunk, MacOsSystemAudioDevice, MacOsSystemAudioError,
    MacOsSystemAudioStopError, SYSTEM_AUDIO_FORMAT,
};

#[cfg(target_os = "macos")]
use crate::MacOsSystemAudioSource;

#[cfg(target_os = "macos")]
const ADAPTER_ID_DOMAIN: &[u8] = b"frame/macos-native-av-adapter/v1\0";
const STARTUP_CALIBRATION_SAMPLES: usize = 5;
const STARTUP_CALIBRATION_TIMEOUT: Duration = Duration::from_millis(750);
const CALIBRATION_IDLE_POLL: Duration = Duration::from_millis(2);
const PERMISSION_PROBE_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(target_os = "macos")]
#[derive(Debug, Error)]
pub enum MacOsNativeAvBridgeCreateError {
    #[error("the macOS system-audio source could not be created")]
    Source(#[source] MacOsSystemAudioError),
    #[error("the macOS native A/V adapter identity could not be derived")]
    AdapterIdentity,
}

trait SystemAudioSource {
    fn device(&mut self) -> MacOsSystemAudioDevice;
    fn request_permission(&mut self) -> PermissionPreflight;
    fn start(&mut self) -> Result<(), MacOsSystemAudioError>;
    fn poll_chunk(&mut self) -> Result<Option<MacOsSystemAudioChunk>, MacOsSystemAudioError>;
    fn stop_and_drain_chunks(
        &mut self,
    ) -> Result<Vec<MacOsSystemAudioChunk>, MacOsSystemAudioStopError>;
    fn is_running(&self) -> bool;
}

#[cfg(target_os = "macos")]
impl SystemAudioSource for MacOsSystemAudioSource {
    fn device(&mut self) -> MacOsSystemAudioDevice {
        Self::device(self)
    }

    fn request_permission(&mut self) -> PermissionPreflight {
        Self::request_permission(self)
    }

    fn start(&mut self) -> Result<(), MacOsSystemAudioError> {
        Self::start(self)
    }

    fn poll_chunk(&mut self) -> Result<Option<MacOsSystemAudioChunk>, MacOsSystemAudioError> {
        Self::poll_chunk(self)
    }

    fn stop_and_drain_chunks(
        &mut self,
    ) -> Result<Vec<MacOsSystemAudioChunk>, MacOsSystemAudioStopError> {
        Self::stop_and_drain_chunks(self)
    }

    fn is_running(&self) -> bool {
        Self::is_running(self)
    }
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

struct OwnedPcmLease {
    bytes: Option<Vec<u8>>,
}

impl AvBufferLease for OwnedPcmLease {
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

struct BufferedChunk {
    chunk: MacOsSystemAudioChunk,
    source_pts_ns: u64,
}

struct SystemAudioBridge<S, P> {
    source: S,
    power: P,
    adapter: AvAdapterInstanceId,
    binding: Option<AvOwnerBinding>,
    device_generation: AvDeviceGeneration,
    catalog_revision: u64,
    control_sequence: u64,
    permission: PermissionState,
    next_permission_probe: Instant,
    active_stamp: Option<AvSourceStamp>,
    source_origin_ns: Option<u64>,
    output_sequence: u64,
    buffered: VecDeque<BufferedChunk>,
    calibration: Option<NativeAvCalibrationBatch>,
    suspended: bool,
    applied_terminal: Option<frame_media::AvTerminalId>,
    applied_terminal_tail: Option<Vec<NativeAvTerminalBuffer>>,
}

impl<S: SystemAudioSource, P: PowerEvents> SystemAudioBridge<S, P> {
    fn new(mut source: S, power: P, adapter: AvAdapterInstanceId) -> Result<Self, NativeAvFailure> {
        let permission = source.device().permission();
        Ok(Self {
            source,
            power,
            adapter,
            binding: None,
            device_generation: AvDeviceGeneration::new(1).map_err(contract_failure)?,
            catalog_revision: 1,
            control_sequence: 0,
            permission,
            next_permission_probe: Instant::now(),
            active_stamp: None,
            source_origin_ns: None,
            output_sequence: 0,
            buffered: VecDeque::new(),
            calibration: None,
            suspended: false,
            applied_terminal: None,
            applied_terminal_tail: None,
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
        let device = self.source.device();
        self.observe_permission(device.permission())?;
        let descriptor = AvDeviceDescriptor::new(
            device.id(),
            self.device_generation,
            AvSourceClass::SystemAudio,
            true,
            self.permission,
            NativeRouteClass::Virtual,
            NativeTimestampKind::HostMonotonic,
            vec![AvFormat::Audio(device.format())],
        )
        .map_err(contract_failure)?;
        AvDeviceCatalog::new(self.adapter, self.catalog_revision, vec![descriptor])
            .map_err(contract_failure)
    }

    fn observe_permission(&mut self, permission: PermissionState) -> Result<(), NativeAvFailure> {
        if permission == self.permission {
            return Ok(());
        }
        self.permission = permission;
        self.catalog_revision = self
            .catalog_revision
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        Ok(())
    }

    fn graph_stamp(
        &mut self,
        ticket: &AvOperationTicket,
        graph: &AvPipelineGraphSpec,
    ) -> Result<Option<AvSourceStamp>, NativeAvFailure> {
        if graph.sources.is_empty() {
            if ticket.stamps().is_empty() {
                return Ok(None);
            }
            return Err(capability_changed());
        }
        let [source] = graph.sources.as_slice() else {
            return Err(capability_changed());
        };
        let [stamp] = ticket.stamps() else {
            return Err(capability_changed());
        };
        if source.class != AvSourceClass::SystemAudio
            || source.device != self.source.device().id()
            || source.generation != self.device_generation
            || source.input_caps
                != frame_media::ExactCapsSpec::Audio(frame_media::AudioCapsSpec {
                    format: SYSTEM_AUDIO_FORMAT,
                    interleaved: true,
                })
            || stamp.class() != AvSourceClass::SystemAudio
            || stamp.generation() != self.device_generation
        {
            return Err(capability_changed());
        }
        Ok(Some(*stamp))
    }

    fn stop_source(&mut self) -> Result<(), NativeAvFailure> {
        self.stop_source_with_tail(false).map(drop)
    }

    fn stop_source_with_tail(
        &mut self,
        retain_tail: bool,
    ) -> Result<Vec<NativeAvTerminalBuffer>, NativeAvFailure> {
        if !self.source.is_running() {
            self.buffered.clear();
            self.calibration = None;
            self.source_origin_ns = None;
            self.output_sequence = 0;
            return Ok(Vec::new());
        }
        match self.source.stop_and_drain_chunks() {
            Ok(tail) => {
                let stamp = self.active_stamp.ok_or_else(capability_changed)?;
                let mut terminal_tail = Vec::new();
                if retain_tail {
                    while let Some(buffered) = self.buffered.pop_front() {
                        terminal_tail.push(self.terminal_chunk(
                            stamp,
                            buffered.chunk,
                            buffered.source_pts_ns,
                        )?);
                    }
                    for chunk in tail {
                        let origin = *self.source_origin_ns.get_or_insert(chunk.source_pts_ns());
                        let source_pts_ns = chunk
                            .source_pts_ns()
                            .checked_sub(origin)
                            .ok_or_else(|| backend_fault(false))?;
                        terminal_tail.push(self.terminal_chunk(stamp, chunk, source_pts_ns)?);
                    }
                } else {
                    self.buffered.clear();
                    drop(tail);
                }
                self.calibration = None;
                self.source_origin_ns = None;
                self.output_sequence = 0;
                Ok(terminal_tail)
            }
            Err(error) if error.capture_teardown_confirmed() => {
                self.buffered.clear();
                self.calibration = None;
                self.source_origin_ns = None;
                self.output_sequence = 0;
                Err(backend_fault(false))
            }
            Err(error) => Err(map_stop_error(error)),
        }
    }

    fn terminal_chunk(
        &mut self,
        stamp: AvSourceStamp,
        chunk: MacOsSystemAudioChunk,
        source_pts_ns: u64,
    ) -> Result<NativeAvTerminalBuffer, NativeAvFailure> {
        self.output_sequence = self
            .output_sequence
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        let timing = NativeAvBufferTiming {
            sequence: self.output_sequence,
            source_pts_ns,
            duration_ns: chunk.duration_ns(),
            arrival: MonotonicTimeNs::new(chunk.arrival_ns()),
            latency: SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Unknown,
            },
            discontinuity: chunk.discontinuity(),
        };
        NativeAvTerminalBuffer::new(
            stamp,
            timing,
            AvFormat::Audio(SYSTEM_AUDIO_FORMAT),
            chunk.into_samples_f32le(),
        )
        .map_err(contract_failure)
    }

    fn start_source(&mut self, stamp: AvSourceStamp) -> Result<(), NativeAvFailure> {
        self.active_stamp = Some(stamp);
        self.source_origin_ns = None;
        self.output_sequence = 0;
        self.buffered.clear();
        self.calibration = None;
        self.suspended = false;
        match self.source.start() {
            Ok(()) => Ok(()),
            Err(error) => {
                if !self.source.is_running() {
                    self.active_stamp = None;
                }
                Err(map_source_error(error))
            }
        }
    }

    fn collect_calibration(
        &mut self,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        if self.active_stamp != Some(stamp) || !self.source.is_running() {
            return Err(capability_changed());
        }
        if let Some(batch) = &self.calibration {
            if batch.stamp() == stamp {
                return Ok(batch.clone());
            }
            return Err(capability_changed());
        }

        let deadline = Instant::now()
            .checked_add(STARTUP_CALIBRATION_TIMEOUT)
            .ok_or_else(|| backend_fault(false))?;
        while self.buffered.len() < STARTUP_CALIBRATION_SAMPLES {
            match self.source.poll_chunk().map_err(map_source_error)? {
                Some(chunk) => {
                    let origin = *self.source_origin_ns.get_or_insert(chunk.source_pts_ns());
                    let source_pts_ns = chunk
                        .source_pts_ns()
                        .checked_sub(origin)
                        .ok_or_else(|| backend_fault(false))?;
                    self.buffered.push_back(BufferedChunk {
                        chunk,
                        source_pts_ns,
                    });
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
        let samples = self
            .buffered
            .iter()
            .take(STARTUP_CALIBRATION_SAMPLES)
            .map(|buffered| CalibrationSample {
                master_arrival: MonotonicTimeNs::new(buffered.chunk.arrival_ns()),
                source_pts_ns: buffered.source_pts_ns,
                latency: SourceLatency {
                    reported_ns: 0,
                    confidence: LatencyConfidence::Unknown,
                },
            })
            .collect();
        let batch = NativeAvCalibrationBatch::new(stamp, samples).map_err(contract_failure)?;
        for _ in 0..STARTUP_CALIBRATION_SAMPLES {
            self.buffered.pop_front();
        }
        self.calibration = Some(batch.clone());
        Ok(batch)
    }

    fn chunk_event(
        &mut self,
        stamp: AvSourceStamp,
        chunk: MacOsSystemAudioChunk,
        source_pts_ns: u64,
    ) -> Result<NativeAvEvent, NativeAvFailure> {
        self.output_sequence = self
            .output_sequence
            .checked_add(1)
            .ok_or_else(|| backend_fault(false))?;
        let timing = NativeAvBufferTiming {
            sequence: self.output_sequence,
            source_pts_ns,
            duration_ns: chunk.duration_ns(),
            arrival: MonotonicTimeNs::new(chunk.arrival_ns()),
            latency: SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Unknown,
            },
            discontinuity: chunk.discontinuity(),
        };
        let bytes = chunk.into_samples_f32le();
        let buffer = NativeAvBuffer::new(
            stamp,
            timing,
            AvFormat::Audio(SYSTEM_AUDIO_FORMAT),
            Box::new(OwnedPcmLease { bytes: Some(bytes) }),
        )
        .map_err(contract_failure)?;
        Ok(NativeAvEvent::Buffer(buffer))
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
                self.stop_source()?;
                self.suspended = true;
                Ok(Some(NativeAvEvent::Sleep))
            }
            Some(SystemPowerEvent::DidWake) => Ok(Some(NativeAvEvent::Wake)),
            None => Ok(None),
        }
    }

    fn poll_permission(&mut self) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let now = Instant::now();
        if now < self.next_permission_probe {
            return Ok(None);
        }
        self.next_permission_probe = now
            .checked_add(PERMISSION_PROBE_INTERVAL)
            .ok_or_else(|| backend_fault(false))?;
        let permission = self.source.device().permission();
        if permission == self.permission {
            return Ok(None);
        }
        self.observe_permission(permission)?;
        if matches!(
            permission,
            PermissionState::Denied | PermissionState::Restricted | PermissionState::Revoked
        ) {
            self.stop_source()?;
        }
        Ok(Some(NativeAvEvent::PermissionChanged {
            stamp: self.next_control_stamp()?,
            class: AvSourceClass::SystemAudio,
            state: permission,
        }))
    }
}

impl<S: SystemAudioSource, P: PowerEvents> NativeAvBridge for SystemAudioBridge<S, P> {
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
        if self.applied_terminal == Some(ticket.terminal_id()) {
            let terminal_tail = self
                .applied_terminal_tail
                .clone()
                .ok_or_else(|| backend_fault(false))?;
            Ok(AvTerminalPostcondition::Applied {
                terminal_id: ticket.terminal_id(),
                terminal_tail,
            })
        } else {
            Ok(AvTerminalPostcondition::NotApplied)
        }
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
            AvNativeRequest::RequestPermission(AvSourceClass::SystemAudio) => {
                let permission = permission_state(self.source.request_permission());
                self.observe_permission(permission)?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::RequestPermission(_) => Err(NativeAvFailure {
                code: NativeAvFailureCode::SourceUnavailable,
                retryable: false,
            }),
            AvNativeRequest::Start(graph) | AvNativeRequest::Reconfigure(graph) => {
                if self.source.is_running() {
                    self.stop_source()?;
                }
                let stamp = self.graph_stamp(&ticket, graph)?;
                match stamp {
                    Some(stamp) => self.start_source(stamp)?,
                    None => self.active_stamp = None,
                }
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Pause => {
                self.stop_source()?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Resume => {
                let [stamp] = ticket.stamps() else {
                    return Err(capability_changed());
                };
                if stamp.class() != AvSourceClass::SystemAudio
                    || stamp.generation() != self.device_generation
                {
                    return Err(capability_changed());
                }
                self.start_source(*stamp)?;
                Ok(ticket.acknowledge(false))
            }
            AvNativeRequest::Stop | AvNativeRequest::Cancel => {
                let terminal = ticket.terminal_id().ok_or_else(|| backend_fault(false))?;
                let retain_tail = matches!(request, AvNativeRequest::Stop);
                self.applied_terminal_tail = None;
                let stopped = self.stop_source_with_tail(retain_tail);
                if !self.source.is_running() {
                    self.active_stamp = None;
                    self.applied_terminal = Some(terminal);
                }
                let tail = stopped?;
                self.applied_terminal_tail = Some(tail.clone());
                if retain_tail {
                    Ok(ticket.acknowledge_with_terminal_tail(true, tail))
                } else {
                    Ok(ticket.acknowledge(true))
                }
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
        if let Some(event) = self.poll_permission()? {
            return Ok(Some(event));
        }
        let Some(stamp) = self.active_stamp else {
            return Ok(None);
        };
        if self.suspended || !self.source.is_running() {
            return Ok(None);
        }
        if let Some(buffered) = self.buffered.pop_front() {
            return self
                .chunk_event(stamp, buffered.chunk, buffered.source_pts_ns)
                .map(Some);
        }
        let Some(chunk) = self.source.poll_chunk().map_err(map_source_error)? else {
            return Ok(None);
        };
        let origin = *self.source_origin_ns.get_or_insert(chunk.source_pts_ns());
        let source_pts_ns = chunk
            .source_pts_ns()
            .checked_sub(origin)
            .ok_or_else(|| backend_fault(false))?;
        self.chunk_event(stamp, chunk, source_pts_ns).map(Some)
    }
}

#[cfg(target_os = "macos")]
pub struct MacOsNativeAvBridge {
    inner: SystemAudioBridge<MacOsSystemAudioSource, NativePowerEvents>,
}

#[cfg(target_os = "macos")]
impl MacOsNativeAvBridge {
    pub fn new(
        installation_secret: [u8; 32],
        power: &SystemPowerMonitor,
    ) -> Result<Self, MacOsNativeAvBridgeCreateError> {
        let adapter = derive_adapter_id(&installation_secret)?;
        let source = MacOsSystemAudioSource::new(installation_secret)
            .map_err(MacOsNativeAvBridgeCreateError::Source)?;
        let inner = SystemAudioBridge::new(source, NativePowerEvents(power.cursor()), adapter)
            .map_err(|_| MacOsNativeAvBridgeCreateError::AdapterIdentity)?;
        Ok(Self { inner })
    }
}

#[cfg(target_os = "macos")]
impl NativeAvBridge for MacOsNativeAvBridge {
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
fn derive_adapter_id(
    installation_secret: &[u8; 32],
) -> Result<AvAdapterInstanceId, MacOsNativeAvBridgeCreateError> {
    if installation_secret.iter().all(|byte| *byte == 0) {
        return Err(MacOsNativeAvBridgeCreateError::AdapterIdentity);
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, installation_secret);
    let digest = hmac::sign(&key, ADAPTER_ID_DOMAIN);
    let mut opaque = [0_u8; 16];
    opaque.copy_from_slice(&digest.as_ref()[..16]);
    AvAdapterInstanceId::from_opaque(opaque)
        .map_err(|_| MacOsNativeAvBridgeCreateError::AdapterIdentity)
}

const fn permission_state(permission: PermissionPreflight) -> PermissionState {
    match permission {
        PermissionPreflight::Granted => PermissionState::Granted,
        PermissionPreflight::PromptRequired => PermissionState::PromptRequired,
        PermissionPreflight::Denied(_) => PermissionState::Denied,
        PermissionPreflight::Restricted => PermissionState::Restricted,
        PermissionPreflight::Revoked(_) => PermissionState::Revoked,
    }
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

const fn map_source_error(error: MacOsSystemAudioError) -> NativeAvFailure {
    let (code, retryable) = match error {
        MacOsSystemAudioError::PermissionDenied => (NativeAvFailureCode::PermissionDenied, false),
        MacOsSystemAudioError::AlreadyRunning
        | MacOsSystemAudioError::NativeOperationCapacityUnavailable => {
            (NativeAvFailureCode::Busy, true)
        }
        MacOsSystemAudioError::ShareableContentUnavailable
        | MacOsSystemAudioError::NoDisplayAvailable
        | MacOsSystemAudioError::UnexpectedStreamStop
        | MacOsSystemAudioError::CallbackQueueDisconnected
        | MacOsSystemAudioError::NotRunning => (NativeAvFailureCode::SourceUnavailable, true),
        MacOsSystemAudioError::NativeOperationTimedOut
        | MacOsSystemAudioError::CaptureStartTeardownUnconfirmed
        | MacOsSystemAudioError::CaptureTeardownUnconfirmed
        | MacOsSystemAudioError::CallbackQueueFenceTimedOut
        | MacOsSystemAudioError::DelegateQuiescenceUnconfirmed => {
            (NativeAvFailureCode::Timeout, true)
        }
        MacOsSystemAudioError::UnexpectedAudioFormat => (NativeAvFailureCode::FormatChanged, true),
        MacOsSystemAudioError::InvalidInstallationSecret
        | MacOsSystemAudioError::DeviceIdDerivationFailed
        | MacOsSystemAudioError::NativeOperationWorkerUnavailable
        | MacOsSystemAudioError::OutputHandlerRegistrationFailed
        | MacOsSystemAudioError::CaptureStartFailed
        | MacOsSystemAudioError::CaptureStopFailed
        | MacOsSystemAudioError::OutputHandlerReleaseUnconfirmed
        | MacOsSystemAudioError::InvalidSampleBuffer
        | MacOsSystemAudioError::MissingAudioBuffer
        | MacOsSystemAudioError::InvalidAudioBufferLayout
        | MacOsSystemAudioError::AudioChunkTooLarge
        | MacOsSystemAudioError::NonFiniteAudioSample
        | MacOsSystemAudioError::InvalidTimestamp
        | MacOsSystemAudioError::SequenceExhausted => (NativeAvFailureCode::BackendFault, false),
    };
    NativeAvFailure { code, retryable }
}

const fn map_stop_error(error: MacOsSystemAudioStopError) -> NativeAvFailure {
    match error {
        MacOsSystemAudioStopError::NativeStopUnconfirmed(source)
        | MacOsSystemAudioStopError::CallbackQuiescenceUnconfirmed(source)
        | MacOsSystemAudioStopError::CaptureFailedAfterTeardown(source) => map_source_error(source),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use frame_media::{
        AvActionExecution, AvCaptureSession, AvRuntimePolicy, AvSessionId, AvSyncPolicy,
        BoundNativeAvBridge, DeviceSelectionV2, NativeAvGstreamerGraph, NativeAvRuntime,
        NativeAvRuntimeOutcome, NativeAvSourceTeardown, prepare_runtime,
    };

    use super::*;

    struct FakeSourceState {
        device: MacOsSystemAudioDevice,
        requested_permission: PermissionPreflight,
        running: bool,
        chunks: VecDeque<MacOsSystemAudioChunk>,
        stop_error: Option<MacOsSystemAudioStopError>,
        starts: u8,
        stops: u8,
    }

    struct FakeSource {
        state: Arc<Mutex<FakeSourceState>>,
    }

    impl FakeSource {
        fn new(permission: PermissionState) -> (Self, Arc<Mutex<FakeSourceState>>) {
            let state = Arc::new(Mutex::new(FakeSourceState {
                device: MacOsSystemAudioDevice {
                    id: frame_media::AvDeviceId::from_opaque([9; 16]).expect("device"),
                    permission,
                },
                requested_permission: PermissionPreflight::Granted,
                running: false,
                chunks: VecDeque::new(),
                stop_error: None,
                starts: 0,
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

    impl SystemAudioSource for FakeSource {
        fn device(&mut self) -> MacOsSystemAudioDevice {
            self.state.lock().expect("fake source").device
        }

        fn request_permission(&mut self) -> PermissionPreflight {
            let mut state = self.state.lock().expect("fake source");
            state.device.permission = permission_state(state.requested_permission);
            state.requested_permission
        }

        fn start(&mut self) -> Result<(), MacOsSystemAudioError> {
            let mut state = self.state.lock().expect("fake source");
            if state.running {
                return Err(MacOsSystemAudioError::AlreadyRunning);
            }
            if state.device.permission != PermissionState::Granted {
                return Err(MacOsSystemAudioError::PermissionDenied);
            }
            state.running = true;
            state.starts = state.starts.saturating_add(1);
            Ok(())
        }

        fn poll_chunk(&mut self) -> Result<Option<MacOsSystemAudioChunk>, MacOsSystemAudioError> {
            let mut state = self.state.lock().expect("fake source");
            if !state.running {
                return Err(MacOsSystemAudioError::NotRunning);
            }
            Ok(state.chunks.pop_front())
        }

        fn stop_and_drain_chunks(
            &mut self,
        ) -> Result<Vec<MacOsSystemAudioChunk>, MacOsSystemAudioStopError> {
            let mut state = self.state.lock().expect("fake source");
            state.stops = state.stops.saturating_add(1);
            if let Some(error) = state.stop_error.take() {
                if error.capture_teardown_confirmed() {
                    state.running = false;
                }
                return Err(error);
            }
            state.running = false;
            Ok(state.chunks.drain(..).collect())
        }

        fn is_running(&self) -> bool {
            self.state.lock().expect("fake source").running
        }
    }

    #[derive(Default)]
    struct FakePower {
        events: VecDeque<SystemPowerEvent>,
        fail: bool,
    }

    impl PowerEvents for FakePower {
        fn poll_power(&mut self) -> Result<Option<SystemPowerEvent>, SystemPowerMonitorError> {
            if self.fail {
                Err(SystemPowerMonitorError::EventGap)
            } else {
                Ok(self.events.pop_front())
            }
        }
    }

    fn chunk(sequence: u64) -> MacOsSystemAudioChunk {
        MacOsSystemAudioChunk {
            sequence,
            source_pts_ns: 8_000_000_000 + (sequence - 1) * 10_000_000,
            arrival_ns: 5_000_000 + (sequence - 1) * 10_000_000,
            duration_ns: 10_000_000,
            discontinuity: sequence == 1,
            samples_f32le: vec![0; 3_840],
        }
    }

    fn adapter() -> AvAdapterInstanceId {
        AvAdapterInstanceId::from_opaque([7; 16]).expect("adapter")
    }

    fn session_id(seed: u8) -> AvSessionId {
        AvSessionId::from_csprng([seed; 16]).expect("session")
    }

    fn settings(device: frame_media::AvDeviceId) -> frame_media::AvCaptureSettingsV2 {
        frame_media::AvCaptureSettingsV2 {
            version: frame_media::AV_SETTINGS_VERSION,
            microphone: DeviceSelectionV2::Disabled,
            system_audio: DeviceSelectionV2::Pinned {
                id: device,
                format: AvFormat::Audio(SYSTEM_AUDIO_FORMAT),
            },
            camera: DeviceSelectionV2::Disabled,
        }
    }

    fn enqueue_calibration(state: &Arc<Mutex<FakeSourceState>>) {
        let mut state = state.lock().expect("fake source");
        for sequence in 1..=7_u64 {
            state.chunks.push_back(chunk(sequence));
        }
    }

    fn started(
        source: FakeSource,
        power: FakePower,
        source_state: Arc<Mutex<FakeSourceState>>,
        seed: u8,
    ) -> (
        BoundNativeAvBridge<SystemAudioBridge<FakeSource, FakePower>>,
        AvCaptureSession,
        Arc<Mutex<FakeSourceState>>,
    ) {
        let bridge = SystemAudioBridge::new(source, power, adapter()).expect("bridge");
        let mut bound = BoundNativeAvBridge::new(bridge, session_id(seed)).expect("bound");
        let mut session = AvCaptureSession::new(bound.claim_session().expect("owner"));
        let capabilities = bound.capabilities().expect("capabilities");
        let catalog = bound.enumerate().expect("catalog");
        let device = catalog.devices()[0].id();
        let action = session
            .request_start(capabilities, catalog, settings(device), false)
            .expect("request start");
        let AvActionExecution::Acknowledged(acknowledgement) = action
            .execute_source(&mut session, &mut bound)
            .expect("execute start")
        else {
            panic!("start must acknowledge");
        };
        session.complete(acknowledgement).expect("complete start");
        (bound, session, source_state)
    }

    #[test]
    fn bridge_calibrates_and_delivers_owned_pcm_in_sequence() {
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let (mut bridge, mut session, _) = started(source, FakePower::default(), source_state, 1);
        let stamp = session
            .source_stamp(AvSourceClass::SystemAudio)
            .expect("stamp");
        let batch = bridge
            .startup_calibration(stamp)
            .expect("calibration batch");
        assert_eq!(batch.samples().len(), STARTUP_CALIBRATION_SAMPLES);
        assert!(
            batch
                .samples()
                .iter()
                .all(|sample| sample.source_pts_ns < 50_000_000)
        );
        session
            .calibrate_source(stamp, AvSyncPolicy::default(), batch.samples())
            .expect("install calibration");

        let outcome = session
            .poll_source(&mut bridge)
            .expect("poll")
            .expect("buffer");
        assert!(matches!(
            outcome.queue,
            Some(frame_media::AvQueuePush::Accepted)
        ));
        let input = session
            .pop_buffer(
                AvSourceClass::SystemAudio,
                MonotonicTimeNs::new(100_000_000),
            )
            .expect("pop")
            .expect("owned buffer")
            .into_appsrc_input()
            .expect("appsrc input");
        assert_eq!(input.payload().bytes().map(<[u8]>::len), Some(3_840));
        input.release();
    }

    #[test]
    fn production_appsrc_runtime_consumes_bridge_pcm_and_confirms_teardown() {
        prepare_runtime().expect("GStreamer runtime");
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let bridge = SystemAudioBridge::new(source, FakePower::default(), adapter())
            .expect("system-audio bridge");
        let mut bound = BoundNativeAvBridge::new(bridge, session_id(6)).expect("bound bridge");
        let mut session = AvCaptureSession::new(bound.claim_session().expect("session owner"));
        let capabilities = bound.capabilities().expect("capabilities");
        let catalog = bound.enumerate().expect("catalog");
        let capture_settings = settings(catalog.devices()[0].id());
        let graph_spec = AvPipelineGraphSpec::negotiate(&catalog, capture_settings, false)
            .expect("graph specification");
        let action = session
            .request_start(capabilities, catalog, capture_settings, false)
            .expect("start request");
        let AvActionExecution::Acknowledged(acknowledgement) = action
            .execute_source(&mut session, &mut bound)
            .expect("start execution")
        else {
            panic!("start acknowledgement");
        };
        session.complete(acknowledgement).expect("complete start");
        let graph = NativeAvGstreamerGraph::build(&graph_spec).expect("native graph");
        let mut runtime = NativeAvRuntime::attach(
            bound,
            session,
            graph,
            AvSyncPolicy::default(),
            AvRuntimePolicy::default(),
        )
        .expect("attach runtime");

        let report = runtime
            .poll(MonotonicTimeNs::new(100_000_000))
            .expect("runtime poll");
        assert_eq!(report.buffers_pushed, 2, "runtime report: {report:?}");
        let termination = runtime.cancel().expect("bounded cancel");
        assert_eq!(termination.outcome, NativeAvRuntimeOutcome::Cancelled);
        assert_eq!(
            termination.source_teardown,
            NativeAvSourceTeardown::Confirmed
        );
        assert_eq!(source_state.lock().expect("fake source").stops, 1);
    }

    #[test]
    fn pause_resume_rotates_epoch_and_recalibrates_without_reusing_samples() {
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let (mut bridge, mut session, source_state) =
            started(source, FakePower::default(), source_state, 2);
        let first = session
            .source_stamp(AvSourceClass::SystemAudio)
            .expect("first stamp");
        let action = session.request_pause().expect("pause");
        let AvActionExecution::Acknowledged(ack) = action
            .execute_source(&mut session, &mut bridge)
            .expect("execute pause")
        else {
            panic!("pause acknowledgement");
        };
        session.complete(ack).expect("complete pause");

        enqueue_calibration(&source_state);
        let capabilities = bridge.capabilities().expect("capabilities");
        let catalog = bridge.enumerate().expect("catalog");
        let action = session
            .request_resume(capabilities, catalog)
            .expect("resume");
        let AvActionExecution::Acknowledged(ack) = action
            .execute_source(&mut session, &mut bridge)
            .expect("execute resume")
        else {
            panic!("resume acknowledgement");
        };
        session.complete(ack).expect("complete resume");
        let second = session
            .source_stamp(AvSourceClass::SystemAudio)
            .expect("second stamp");
        assert!(second.stream_epoch().get() > first.stream_epoch().get());
        assert_eq!(
            bridge
                .startup_calibration(second)
                .expect("second calibration")
                .stamp(),
            second
        );
    }

    #[test]
    fn sleep_stops_native_authority_and_requires_an_explicit_resume() {
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let mut power = FakePower::default();
        power.events.push_back(SystemPowerEvent::WillSleep);
        power.events.push_back(SystemPowerEvent::DidWake);
        let (mut bridge, mut session, source_state) = started(source, power, source_state, 3);

        let outcome = session
            .poll_source(&mut bridge)
            .expect("sleep poll")
            .expect("sleep event");
        assert_eq!(session.state(), frame_media::AvSessionState::Suspended);
        assert_eq!(
            outcome.diagnostics[0].code,
            frame_media::AvStableCode::Sleep
        );
        assert!(!source_state.lock().expect("fake source").running);
        let wake = session
            .poll_source(&mut bridge)
            .expect("wake poll")
            .expect("wake event");
        assert_eq!(wake.diagnostics[0].code, frame_media::AvStableCode::Wake);
        assert!(!source_state.lock().expect("fake source").running);
    }

    #[test]
    fn terminal_reconciliation_does_not_release_native_authority_twice() {
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let (mut bridge, mut session, source_state) =
            started(source, FakePower::default(), source_state, 4);
        let stamp = session
            .source_stamp(AvSourceClass::SystemAudio)
            .expect("system-audio stamp");
        let calibration = bridge
            .startup_calibration(stamp)
            .expect("startup calibration");
        session
            .calibrate_source(stamp, AvSyncPolicy::default(), calibration.samples())
            .expect("install calibration");
        let action = session.request_stop().expect("stop").expect("stop action");
        let AvActionExecution::Acknowledged(delayed_acknowledgement) = action
            .execute_source(&mut session, &mut bridge)
            .expect("stop dispatch")
        else {
            panic!("first stop must acknowledge");
        };
        let retry = session.retry_teardown().expect("retry");
        let AvActionExecution::Acknowledged(ack) = retry
            .execute_source(&mut session, &mut bridge)
            .expect("reconcile")
        else {
            panic!("reconcile must acknowledge applied terminal");
        };
        let terminal = session.complete(ack).expect("complete reconciled stop");
        drop(delayed_acknowledgement);
        assert_eq!(terminal.len(), 2);
        assert_eq!(terminal[0].sequence(), 1);
        assert_eq!(terminal[1].sequence(), 2);
        for buffer in terminal {
            buffer
                .into_appsrc_input()
                .expect("authenticated terminal PCM")
                .release();
        }
        assert_eq!(source_state.lock().expect("fake source").stops, 1);
        assert_eq!(session.state(), frame_media::AvSessionState::Stopped);
    }

    #[test]
    fn permission_denial_is_owner_stamped_and_disables_only_optional_audio() {
        let (source, source_state) = FakeSource::new(PermissionState::Granted);
        enqueue_calibration(&source_state);
        let (mut bridge, mut session, source_state) =
            started(source, FakePower::default(), source_state, 5);
        source_state.lock().expect("fake source").device.permission = PermissionState::Revoked;
        let outcome = session
            .poll_source(&mut bridge)
            .expect("permission poll")
            .expect("permission event");
        assert_eq!(outcome.disabled_sources, vec![AvSourceClass::SystemAudio]);
        assert!(outcome.native_reconfigure_required);
        assert!(!source_state.lock().expect("fake source").running);
    }
}
