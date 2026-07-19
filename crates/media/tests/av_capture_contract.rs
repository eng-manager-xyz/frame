use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use frame_media::*;

fn opaque(seed: u8) -> [u8; 16] {
    [seed; 16]
}

fn adapter_id(seed: u8) -> AvAdapterInstanceId {
    AvAdapterInstanceId::from_opaque(opaque(seed)).expect("adapter ID")
}

fn device_id(seed: u8) -> AvDeviceId {
    AvDeviceId::from_opaque(opaque(seed)).expect("device ID")
}

fn session_id(seed: u8) -> AvSessionId {
    AvSessionId::from_csprng(opaque(seed)).expect("session ID")
}

fn generation(value: u64) -> AvDeviceGeneration {
    AvDeviceGeneration::new(value).expect("generation")
}

fn audio_format(rate: u32, channels: u8) -> AvFormat {
    AvFormat::Audio(AudioFormat {
        sample_rate: rate,
        channels,
        sample_format: AudioSampleFormat::Float32,
    })
}

fn camera_format(width: u32, height: u32, rate: u32) -> AvFormat {
    AvFormat::Camera(CameraFormat {
        width,
        height,
        frame_rate_numerator: rate,
        frame_rate_denominator: 1,
        pixel_format: PixelFormat::Nv12,
    })
}

fn device(
    seed: u8,
    class: AvSourceClass,
    default: bool,
    permission: PermissionState,
    format: AvFormat,
) -> AvDeviceDescriptor {
    AvDeviceDescriptor::new(
        device_id(seed),
        generation(1),
        class,
        default,
        permission,
        if class == AvSourceClass::Microphone {
            NativeRouteClass::WirelessWideband
        } else {
            NativeRouteClass::BuiltIn
        },
        NativeTimestampKind::DeviceMonotonic,
        vec![format],
    )
    .expect("device")
}

fn capabilities(adapter: AvAdapterInstanceId) -> NativeAvBridgeCapabilities {
    NativeAvBridgeCapabilities {
        contract_version: AV_CAPTURE_CONTRACT_VERSION,
        adapter,
        permission_prompt: true,
        hotplug_events: true,
        default_change_events: true,
        sleep_wake_events: true,
        bounded_nonblocking_ingress: true,
        explicit_timestamps: true,
        discontinuity_signaling: true,
        latency_reporting: true,
    }
}

fn full_catalog(adapter: AvAdapterInstanceId) -> AvDeviceCatalog {
    AvDeviceCatalog::new(
        adapter,
        1,
        vec![
            device(
                10,
                AvSourceClass::Microphone,
                true,
                PermissionState::Granted,
                audio_format(48_000, 1),
            ),
            device(
                11,
                AvSourceClass::SystemAudio,
                true,
                PermissionState::Granted,
                audio_format(48_000, 2),
            ),
            device(
                12,
                AvSourceClass::Camera,
                true,
                PermissionState::Granted,
                camera_format(1_920, 1_080, 30),
            ),
        ],
    )
    .expect("catalog")
}

fn full_settings() -> AvCaptureSettingsV2 {
    AvCaptureSettingsV2 {
        version: AV_SETTINGS_VERSION,
        microphone: DeviceSelectionV2::Pinned {
            id: device_id(10),
            format: audio_format(48_000, 1),
        },
        system_audio: DeviceSelectionV2::Pinned {
            id: device_id(11),
            format: audio_format(48_000, 2),
        },
        camera: DeviceSelectionV2::Pinned {
            id: device_id(12),
            format: camera_format(1_920, 1_080, 30),
        },
    }
}

#[derive(Debug, Default)]
struct FakeState {
    binding: Option<AvOwnerBinding>,
    events: VecDeque<NativeAvEvent>,
    operations: Vec<AvOperationKind>,
    stamps: Vec<AvSourceStamp>,
    predecessor_stamps: Vec<AvSourceStamp>,
    fail_next: Option<NativeAvFailure>,
    fail_capabilities: Option<NativeAvFailure>,
    mutate_catalog_on_execute: bool,
    reconciliations: usize,
    terminal_applied: Option<AvTerminalId>,
    native_release_count: usize,
    last_native_timeout: Option<std::time::Duration>,
}

#[derive(Debug)]
struct FakeBridge {
    adapter: AvAdapterInstanceId,
    capabilities: NativeAvBridgeCapabilities,
    catalog: AvDeviceCatalog,
    state: Arc<Mutex<FakeState>>,
}

impl FakeBridge {
    fn new(
        adapter: AvAdapterInstanceId,
        catalog: AvDeviceCatalog,
    ) -> (Self, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        (
            Self {
                adapter,
                capabilities: capabilities(adapter),
                catalog,
                state: Arc::clone(&state),
            },
            state,
        )
    }
}

impl NativeAvBridge for FakeBridge {
    fn adapter_instance(&self) -> AvAdapterInstanceId {
        self.adapter
    }

    fn bind(&mut self, ticket: AvSessionClaimTicket) -> Result<AvOwnerBinding, NativeAvFailure> {
        let binding = ticket.accept();
        self.state.lock().expect("fake state").binding = Some(binding);
        Ok(binding)
    }

    fn capabilities(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<NativeAvBridgeCapabilities, NativeAvFailure> {
        let mut state = self.state.lock().expect("fake state");
        assert_eq!(state.binding, Some(ticket.binding()));
        if let Some(failure) = state.fail_capabilities.take() {
            return Err(failure);
        }
        Ok(self.capabilities)
    }

    fn enumerate(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<AvDeviceCatalog, NativeAvFailure> {
        assert_eq!(
            self.state.lock().expect("fake state").binding,
            Some(ticket.binding())
        );
        if self
            .state
            .lock()
            .expect("fake state")
            .mutate_catalog_on_execute
        {
            return AvDeviceCatalog::new(self.adapter, 2, self.catalog.devices().to_vec()).map_err(
                |_| NativeAvFailure {
                    code: NativeAvFailureCode::BackendFault,
                    retryable: false,
                },
            );
        }
        Ok(self.catalog.clone())
    }

    fn startup_calibration(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        assert_eq!(
            self.state.lock().expect("fake state").binding,
            Some(ticket.binding())
        );
        NativeAvCalibrationBatch::new(stamp, session_calibration_samples()).map_err(|_| {
            NativeAvFailure {
                code: NativeAvFailureCode::BackendFault,
                retryable: false,
            }
        })
    }

    fn reconcile_terminal(
        &mut self,
        ticket: AvTerminalReconcileTicket,
    ) -> Result<AvTerminalPostcondition, NativeAvFailure> {
        let mut state = self.state.lock().expect("fake state");
        assert_eq!(state.binding, Some(ticket.owner()));
        assert!(matches!(
            ticket.kind(),
            AvOperationKind::Stop | AvOperationKind::Cancel
        ));
        assert!(!ticket.native_timeout().is_zero());
        assert!(ticket.native_timeout() <= MAX_OPERATION_TIMEOUT);
        state.reconciliations += 1;
        state.last_native_timeout = Some(ticket.native_timeout());
        Ok(if state.terminal_applied == Some(ticket.terminal_id()) {
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
        assert_eq!(ticket.kind(), request.kind());
        let mut state = self.state.lock().expect("fake state");
        state.operations.push(request.kind());
        state.stamps = ticket.stamps().to_vec();
        state.predecessor_stamps = ticket.predecessor_stamps().to_vec();
        state.last_native_timeout = Some(ticket.native_timeout());
        if let Some(failure) = state.fail_next.take() {
            return Err(failure);
        }
        if matches!(request, AvNativeRequest::Stop | AvNativeRequest::Cancel) {
            state.terminal_applied = ticket.terminal_id();
            state.native_release_count += 1;
        }
        Ok(ticket.acknowledge(matches!(
            request,
            AvNativeRequest::Stop | AvNativeRequest::Cancel
        )))
    }

    fn poll(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<Option<NativeAvEvent>, NativeAvFailure> {
        let mut state = self.state.lock().expect("fake state");
        assert_eq!(state.binding, Some(ticket.binding()));
        Ok(state.events.pop_front())
    }
}

fn new_session(source: &mut BoundNativeAvBridge<FakeBridge>) -> AvCaptureSession {
    AvCaptureSession::new(source.claim_session().expect("one session owner"))
}

fn setup_started(
    seed: u8,
) -> (
    BoundNativeAvBridge<FakeBridge>,
    Arc<Mutex<FakeState>>,
    AvCaptureSession,
) {
    let adapter = adapter_id(seed);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(seed)).expect("bound source");
    let mut session = new_session(&mut source);
    let action = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start action");
    let execution = action
        .execute_source(&mut session, &mut source)
        .expect("start dispatch");
    let AvActionExecution::Acknowledged(acknowledgement) = execution else {
        panic!("expected acknowledgement");
    };
    session.complete(acknowledgement).expect("complete start");
    for class in [
        AvSourceClass::Microphone,
        AvSourceClass::SystemAudio,
        AvSourceClass::Camera,
    ] {
        let stamp = session.source_stamp(class).expect("active source stamp");
        session
            .calibrate_source(
                stamp,
                AvSyncPolicy::default(),
                &session_calibration_samples(),
            )
            .expect("source calibration");
    }
    (source, handle, session)
}

#[derive(Debug)]
struct CountingLease {
    bytes: u64,
    payload: Option<AvPayloadBody>,
    released: Arc<AtomicUsize>,
}

impl AvBufferLease for CountingLease {
    fn retained_bytes(&self) -> u64 {
        self.bytes
    }

    fn take_payload(&mut self) -> Option<AvPayloadBody> {
        self.payload.take()
    }

    fn release(self: Box<Self>) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

fn lease(bytes: u64, released: &Arc<AtomicUsize>) -> Box<dyn AvBufferLease> {
    Box::new(CountingLease {
        bytes,
        payload: Some(AvPayloadBody::Bytes(vec![
            0;
            usize::try_from(bytes)
                .expect("test payload size")
        ])),
        released: Arc::clone(released),
    })
}

fn opaque_lease<T: std::any::Any + Send>(
    bytes: u64,
    value: T,
    released: &Arc<AtomicUsize>,
) -> Box<dyn AvBufferLease> {
    Box::new(CountingLease {
        bytes,
        payload: Some(AvPayloadBody::Opaque(Box::new(value))),
        released: Arc::clone(released),
    })
}

fn session_calibration_samples() -> Vec<CalibrationSample> {
    (0..7_u64)
        .map(|index| CalibrationSample {
            master_arrival: MonotonicTimeNs::new(105_000_000 + index * 10_000_000),
            source_pts_ns: index * 10_000_000,
            latency: SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
        })
        .collect()
}

fn native_timing(
    sequence: u64,
    source_pts_ns: u64,
    duration_ns: u64,
    arrival_ns: u64,
) -> NativeAvBufferTiming {
    NativeAvBufferTiming {
        sequence,
        source_pts_ns,
        duration_ns,
        arrival: MonotonicTimeNs::new(arrival_ns),
        latency: SourceLatency {
            reported_ns: 5_000_000,
            confidence: LatencyConfidence::Measured,
        },
        discontinuity: false,
    }
}

fn control_stamp(
    source: &BoundNativeAvBridge<FakeBridge>,
    revision: u64,
    sequence: u64,
) -> AvControlEventStamp {
    AvControlEventStamp::new(source.binding(), revision, sequence).expect("control stamp")
}

#[derive(Default)]
struct HoldingAppSrc {
    inputs: Vec<AvAppSrcInput>,
}

impl AvLocalAppSrcAdapter for HoldingAppSrc {
    fn push(&mut self, input: AvAppSrcInput) -> Result<(), AvAppSrcPushFailure> {
        self.inputs.push(input);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OpaqueTestHandle(u64);

struct HostileSizeLease {
    size_calls: Arc<AtomicUsize>,
    take_calls: Arc<AtomicUsize>,
    released: Arc<AtomicUsize>,
    payload: Option<AvPayloadBody>,
}

impl AvBufferLease for HostileSizeLease {
    fn retained_bytes(&self) -> u64 {
        let call = self.size_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 { 64 } else { MAX_AV_QUEUE_BYTES }
    }

    fn take_payload(&mut self) -> Option<AvPayloadBody> {
        self.take_calls.fetch_add(1, Ordering::SeqCst);
        self.payload.take()
    }

    fn release(self: Box<Self>) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn local_appsrc_receives_bytes_and_opaque_handles_and_releases_each_exactly_once() {
    let (mut source, handle, mut session) = setup_started(29);
    let stamp = session
        .source_stamp(AvSourceClass::Microphone)
        .expect("microphone stamp");
    let released = Arc::new(AtomicUsize::new(0));
    let buffers = [
        NativeAvBuffer::new(
            stamp,
            native_timing(1, 70_000_000, 10_000_000, 175_000_000),
            audio_format(48_000, 1),
            Box::new(CountingLease {
                bytes: 4,
                payload: Some(AvPayloadBody::Bytes(vec![1, 2, 3, 4])),
                released: Arc::clone(&released),
            }),
        )
        .expect("byte buffer"),
        NativeAvBuffer::new(
            stamp,
            native_timing(2, 80_000_000, 10_000_000, 185_000_000),
            audio_format(48_000, 1),
            opaque_lease(128, OpaqueTestHandle(77), &released),
        )
        .expect("opaque buffer"),
    ];
    for buffer in buffers {
        handle
            .lock()
            .expect("fake state")
            .events
            .push_back(NativeAvEvent::Buffer(buffer));
        session
            .poll_source(&mut source)
            .expect("poll")
            .expect("buffer event");
    }
    let mut appsrc = HoldingAppSrc::default();
    for _ in 0..2 {
        let buffer = session
            .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(185_000_000))
            .expect("pop")
            .expect("queued buffer");
        appsrc
            .push(buffer.into_appsrc_input().expect("corrected appsrc input"))
            .expect("fake appsrc push");
    }
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert_eq!(appsrc.inputs[0].payload().bytes(), Some(&[1, 2, 3, 4][..]));
    assert_eq!(appsrc.inputs[0].payload().retained_bytes(), 4);
    assert_eq!(
        appsrc.inputs[1].payload().opaque::<OpaqueTestHandle>(),
        Some(&OpaqueTestHandle(77))
    );
    let consumed = appsrc.inputs.remove(0);
    consumed.release();
    assert_eq!(released.load(Ordering::SeqCst), 1);
    drop(appsrc);
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

#[test]
fn retained_size_is_snapshotted_once_even_for_a_hostile_mutating_lease() {
    let (mut source, handle, mut session) = setup_started(30);
    let stamp = session
        .source_stamp(AvSourceClass::Microphone)
        .expect("microphone stamp");
    let size_calls = Arc::new(AtomicUsize::new(0));
    let take_calls = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let buffer = NativeAvBuffer::new(
        stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        Box::new(HostileSizeLease {
            size_calls: Arc::clone(&size_calls),
            take_calls: Arc::clone(&take_calls),
            released: Arc::clone(&released),
            payload: Some(AvPayloadBody::Bytes(vec![9; 64])),
        }),
    )
    .expect("buffer");
    assert_eq!(size_calls.load(Ordering::SeqCst), 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(buffer));
    session
        .poll_source(&mut source)
        .expect("poll")
        .expect("buffer event");
    let buffer = session
        .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(175_000_000))
        .expect("pop")
        .expect("queued buffer");
    assert_eq!(buffer.retained_bytes(), 64);
    let input = buffer.into_appsrc_input().expect("appsrc input");
    assert_eq!(input.payload().retained_bytes(), 64);
    assert_eq!(size_calls.load(Ordering::SeqCst), 1);
    assert_eq!(take_calls.load(Ordering::SeqCst), 1);
    input.release();
    assert_eq!(released.load(Ordering::SeqCst), 1);

    let mismatched = NativeAvBuffer::new(
        stamp,
        native_timing(2, 80_000_000, 10_000_000, 185_000_000),
        audio_format(48_000, 1),
        Box::new(CountingLease {
            bytes: 65,
            payload: Some(AvPayloadBody::Bytes(vec![0; 64])),
            released: Arc::clone(&released),
        }),
    )
    .expect("mismatched body buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(mismatched));
    session
        .poll_source(&mut source)
        .expect("poll mismatch")
        .expect("buffer event");
    let mismatched = session
        .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(185_000_000))
        .expect("pop mismatch")
        .expect("queued mismatch");
    assert!(matches!(
        mismatched.into_appsrc_input(),
        Err(AvCaptureError::PayloadSizeMismatch)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

#[test]
fn identities_and_debug_output_are_privacy_safe() {
    assert!(AvDeviceId::from_opaque([0; 16]).is_err());
    let id = device_id(44);
    assert_eq!(format!("{id:?}"), "AvDeviceId(<redacted>)");
    let descriptor = device(
        44,
        AvSourceClass::Microphone,
        true,
        PermissionState::Granted,
        audio_format(48_000, 1),
    );
    let output = format!("{descriptor:?}");
    assert!(!output.contains("44"));
    assert!(!output.contains("serial"));
    assert!(!output.contains("label"));
}

#[test]
fn a_bound_adapter_can_mint_exactly_one_session_owner() {
    let adapter = adapter_id(26);
    let (fake, _handle) = FakeBridge::new(adapter, full_catalog(adapter));
    let mut source = BoundNativeAvBridge::new(fake, session_id(26)).expect("source");
    let owner = source.claim_session().expect("first owner");
    assert!(matches!(
        source.claim_session(),
        Err(AvCaptureError::SessionAlreadyClaimed)
    ));
    let session = AvCaptureSession::new(owner);
    assert_eq!(session.owner(), source.binding());
}

#[test]
fn native_operation_deadline_policy_is_bounded_and_reaches_the_adapter() {
    let adapter = adapter_id(41);
    let (fake, _handle) = FakeBridge::new(adapter, full_catalog(adapter));
    let mut source = BoundNativeAvBridge::new(fake, session_id(41)).expect("source");
    assert!(matches!(
        AvCaptureSession::new_with_policy(
            source.claim_session().expect("owner"),
            AvOperationPolicy {
                native_timeout: std::time::Duration::ZERO,
            },
        ),
        Err(AvCaptureError::InvalidOperationPolicy)
    ));

    let adapter = adapter_id(42);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(42)).expect("source");
    let timeout = std::time::Duration::from_secs(7);
    let mut session = AvCaptureSession::new_with_policy(
        source.claim_session().expect("owner"),
        AvOperationPolicy {
            native_timeout: timeout,
        },
    )
    .expect("policy");
    let start = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start");
    let AvActionExecution::Acknowledged(ack) = start
        .execute_source(&mut session, &mut source)
        .expect("dispatch")
    else {
        panic!("start must acknowledge");
    };
    session.complete(ack).expect("complete start");
    assert_eq!(
        handle.lock().expect("fake state").last_native_timeout,
        Some(timeout)
    );

    let too_long = AvOperationPolicy {
        native_timeout: MAX_OPERATION_TIMEOUT + std::time::Duration::from_nanos(1),
    };
    assert!(matches!(
        too_long.validate(),
        Err(AvCaptureError::InvalidOperationPolicy)
    ));
}

#[test]
fn descriptor_validation_rejects_wrong_classes_duplicates_and_unsafe_formats() {
    assert!(matches!(
        AvDeviceDescriptor::new(
            device_id(1),
            generation(1),
            AvSourceClass::Camera,
            false,
            PermissionState::Granted,
            NativeRouteClass::BuiltIn,
            NativeTimestampKind::HostMonotonic,
            vec![audio_format(48_000, 2)],
        ),
        Err(AvCaptureError::FormatClassMismatch)
    ));
    assert!(matches!(
        AvDeviceDescriptor::new(
            device_id(1),
            generation(1),
            AvSourceClass::Microphone,
            false,
            PermissionState::Granted,
            NativeRouteClass::BuiltIn,
            NativeTimestampKind::HostMonotonic,
            vec![audio_format(48_000, 2), audio_format(48_000, 2)],
        ),
        Err(AvCaptureError::DuplicateFormat)
    ));
    assert!(matches!(
        camera_format(20_000, 20_000, 30).validate_for(AvSourceClass::Camera),
        Err(AvCaptureError::FormatTooLarge)
    ));
    assert!(
        audio_format(7_999, 2)
            .validate_for(AvSourceClass::Microphone)
            .is_err()
    );
}

#[test]
fn catalog_rejects_duplicate_devices_and_defaults() {
    let adapter = adapter_id(1);
    let first = device(
        1,
        AvSourceClass::Camera,
        true,
        PermissionState::Granted,
        camera_format(640, 480, 30),
    );
    assert!(matches!(
        AvDeviceCatalog::new(adapter, 1, vec![first.clone(), first]),
        Err(AvCaptureError::DuplicateDevice)
    ));
    assert!(matches!(
        AvDeviceCatalog::new(
            adapter,
            1,
            vec![
                device(
                    1,
                    AvSourceClass::Camera,
                    true,
                    PermissionState::Granted,
                    camera_format(640, 480, 30),
                ),
                device(
                    2,
                    AvSourceClass::Camera,
                    true,
                    PermissionState::Granted,
                    camera_format(640, 480, 30),
                ),
            ],
        ),
        Err(AvCaptureError::MultipleDefaults)
    ));
}

#[test]
fn legacy_default_migration_never_authorizes_a_changed_sensitive_device() {
    let legacy = LegacyAvCaptureSettingsV1 {
        microphone: LegacyDeviceSelectionV1 {
            id: Some(device_id(10)),
            format: Some(audio_format(48_000, 1)),
            followed_default: true,
        },
        system_audio: LegacyDeviceSelectionV1 {
            id: None,
            format: None,
            followed_default: false,
        },
        camera: LegacyDeviceSelectionV1 {
            id: None,
            format: None,
            followed_default: false,
        },
    };
    let settings = legacy.migrate().expect("migration");
    let adapter = adapter_id(1);
    let changed = AvDeviceCatalog::new(
        adapter,
        2,
        vec![device(
            99,
            AvSourceClass::Microphone,
            true,
            PermissionState::Granted,
            audio_format(48_000, 1),
        )],
    )
    .expect("changed catalog");
    assert_eq!(
        resolve_selection(&changed, AvSourceClass::Microphone, settings.microphone)
            .expect("resolution"),
        SelectionResolution::ConfirmationRequired {
            candidate: device_id(99)
        }
    );
}

#[test]
fn pinned_settings_survive_rename_equivalent_catalogs_but_not_missing_devices() {
    let adapter = adapter_id(1);
    let catalog = full_catalog(adapter);
    assert!(matches!(
        resolve_selection(
            &catalog,
            AvSourceClass::Microphone,
            full_settings().microphone
        )
        .expect("resolution"),
        SelectionResolution::Ready { id, .. } if id == device_id(10)
    ));
    let empty = AvDeviceCatalog::new(adapter, 2, vec![]).expect("empty catalog");
    assert_eq!(
        resolve_selection(
            &empty,
            AvSourceClass::Microphone,
            full_settings().microphone
        )
        .expect("resolution"),
        SelectionResolution::Unavailable
    );
}

#[derive(Default)]
struct MemorySettingsStorage {
    encoded: Option<Vec<u8>>,
}

impl AvSettingsStorage for MemorySettingsStorage {
    fn load(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, AvSettingsStorageError> {
        assert_eq!(max_bytes, MAX_PERSISTED_AV_SETTINGS_BYTES);
        Ok(self.encoded.clone())
    }

    fn store(&mut self, encoded: &[u8]) -> Result<(), AvSettingsStorageError> {
        self.encoded = Some(encoded.to_vec());
        Ok(())
    }
}

#[test]
fn versioned_settings_round_trip_across_restart_without_labels_or_reselection() {
    let mut settings = full_settings();
    settings.microphone = DeviceSelectionV2::FollowDefault {
        format: audio_format(48_000, 1),
        allow_default_changes: false,
        confirmed_id: Some(device_id(10)),
    };
    let mut storage = MemorySettingsStorage::default();
    store_persisted_av_settings(&mut storage, settings).expect("store settings");
    let encoded = storage.encoded.as_ref().expect("encoded settings");
    assert!(encoded.len() <= MAX_PERSISTED_AV_SETTINGS_BYTES);
    let text = std::str::from_utf8(encoded).expect("utf8 codec");
    assert!(!text.contains("label"));
    assert!(!text.contains("microphone name"));

    let mut restarted_storage = MemorySettingsStorage {
        encoded: storage.encoded,
    };
    let restored = load_persisted_av_settings(&mut restarted_storage)
        .expect("load settings")
        .expect("stored value");
    assert_eq!(restored, settings);
    assert!(matches!(
        resolve_selection(
            &full_catalog(adapter_id(31)),
            AvSourceClass::Microphone,
            restored.microphone,
        )
        .expect("same opaque device resolution"),
        SelectionResolution::Ready { id, .. } if id == device_id(10)
    ));

    let changed_default = AvDeviceCatalog::new(
        adapter_id(31),
        2,
        vec![device(
            99,
            AvSourceClass::Microphone,
            true,
            PermissionState::Granted,
            audio_format(48_000, 1),
        )],
    )
    .expect("changed default");
    assert!(matches!(
        resolve_selection(
            &changed_default,
            AvSourceClass::Microphone,
            restored.microphone,
        )
        .expect("safe default resolution"),
        SelectionResolution::ConfirmationRequired { candidate } if candidate == device_id(99)
    ));
}

#[test]
fn settings_codec_rejects_unknown_versions_malformed_ids_fields_and_oversize() {
    let valid = AvSettingsCodec::encode(full_settings()).expect("valid encoding");
    let valid_text = std::str::from_utf8(&valid).expect("utf8");
    let unknown_version = valid_text.replacen("version=2", "version=99", 1);
    assert!(matches!(
        AvSettingsCodec::decode(unknown_version.as_bytes()),
        Err(AvCaptureError::SettingsVersionMismatch)
    ));
    let malformed_id = valid_text.replacen(&device_id(10).to_persisted_hex(), "not-an-id", 1);
    assert!(matches!(
        AvSettingsCodec::decode(malformed_id.as_bytes()),
        Err(AvCaptureError::MalformedPersistedSettings)
    ));
    let unknown_field = format!("{valid_text}\nfuture=true");
    assert!(matches!(
        AvSettingsCodec::decode(unknown_field.as_bytes()),
        Err(AvCaptureError::MalformedPersistedSettings)
    ));
    assert!(matches!(
        AvSettingsCodec::decode(&vec![b'x'; MAX_PERSISTED_AV_SETTINGS_BYTES + 1]),
        Err(AvCaptureError::PersistedSettingsTooLarge)
    ));
}

#[test]
fn graph_negotiates_exact_nonblocking_appsrc_paths() {
    let graph = AvPipelineGraphSpec::negotiate(&full_catalog(adapter_id(1)), full_settings(), true)
        .expect("graph");
    let microphone = graph
        .source(AvSourceClass::Microphone)
        .expect("microphone graph");
    assert_eq!(
        microphone.input_caps,
        ExactCapsSpec::Audio(AudioCapsSpec {
            format: AudioFormat {
                sample_rate: 48_000,
                channels: 1,
                sample_format: AudioSampleFormat::Float32,
            },
            interleaved: true,
        })
    );
    assert!(microphone.elements.contains(&GstElementFamily::AppSrc));
    assert!(!microphone.elements.contains(&GstElementFamily::AudioMixer));
    assert!(!microphone.queue.producer_blocks);
    assert!(microphone.appsrc.is_live);
    assert!(!microphone.appsrc.do_timestamp);
    assert!(!microphone.appsrc.block);
    assert!(microphone.appsrc.time_format_nanoseconds);
    assert_eq!(
        microphone.appsrc.timestamp_mode,
        AppSrcTimestampMode::ExplicitMasterCorrected
    );
    assert!(
        microphone
            .appsrc
            .retain_native_lease_until_downstream_release
    );
    let camera = graph.source(AvSourceClass::Camera).expect("camera graph");
    assert!(camera.elements.contains(&GstElementFamily::VideoConvert));
    assert!(!camera.elements.contains(&GstElementFamily::Tee));
    let mixer = graph.shared_audio_mixer.as_ref().expect("shared mixer");
    assert_eq!(mixer.element, GstElementFamily::AudioMixer);
    assert_eq!(mixer.request_pads.len(), 2);
    assert_ne!(mixer.request_pads[0].pad, mixer.request_pads[1].pad);
    let tee = graph.camera_tee.as_ref().expect("camera tee");
    assert_eq!(tee.element, GstElementFamily::Tee);
    assert!(!tee.record_branch.is_empty());
    assert!(
        tee.preview_branch
            .as_ref()
            .is_some_and(|branch| !branch.is_empty())
    );
    assert!(graph.camera_preview_enabled);
}

#[test]
fn no_optional_device_settings_produce_a_valid_screen_only_graph() {
    let adapter = adapter_id(1);
    let empty = AvDeviceCatalog::new(adapter, 1, vec![]).expect("empty catalog");
    let graph = AvPipelineGraphSpec::negotiate(&empty, AvCaptureSettingsV2::screen_only(), true)
        .expect("screen-only graph");
    assert!(graph.sources.is_empty());
    assert!(!graph.camera_preview_enabled);
}

#[test]
fn missing_mandatory_bridge_capability_fails_closed() {
    let mut value = capabilities(adapter_id(1));
    value.discontinuity_signaling = false;
    assert!(matches!(
        value.validate(),
        Err(AvCaptureError::MissingBridgeCapability)
    ));
}

#[test]
fn one_shot_start_revalidates_the_complete_live_snapshot() {
    let adapter = adapter_id(2);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(2)).expect("bound source");
    let mut session = new_session(&mut source);
    let action = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start action");
    handle.lock().expect("fake state").mutate_catalog_on_execute = true;
    let execution = action
        .execute_source(&mut session, &mut source)
        .expect("bound failure");
    let AvActionExecution::Failed(failure) = execution else {
        panic!("snapshot mutation must fail");
    };
    let failure = session.complete_failure(failure).expect("complete failure");
    assert_eq!(failure.code, NativeAvFailureCode::CapabilityChanged);
    assert_eq!(session.state(), AvSessionState::Idle);
    assert!(handle.lock().expect("fake state").operations.is_empty());
}

#[test]
fn superseded_start_action_cannot_reach_the_native_bridge() {
    let adapter = adapter_id(3);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(3)).expect("bound source");
    let mut session = new_session(&mut source);
    let stale = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start action");
    let stop = session
        .request_stop()
        .expect("stop request")
        .expect("stop action");
    assert!(matches!(
        stale.execute_source(&mut session, &mut source),
        Err(AvCaptureError::StaleOperation)
    ));
    let execution = stop
        .execute_source(&mut session, &mut source)
        .expect("stop dispatch");
    let AvActionExecution::Acknowledged(acknowledgement) = execution else {
        panic!("expected stop acknowledgement");
    };
    session.complete(acknowledgement).expect("stop complete");
    assert_eq!(session.state(), AvSessionState::Stopped);
    assert_eq!(
        handle.lock().expect("fake state").operations,
        vec![AvOperationKind::Stop]
    );
}

#[test]
fn cross_session_action_is_rejected_before_native_dispatch() {
    let adapter_a = adapter_id(4);
    let catalog_a = full_catalog(adapter_a);
    let (fake_a, _handle_a) = FakeBridge::new(adapter_a, catalog_a.clone());
    let mut source_a = BoundNativeAvBridge::new(fake_a, session_id(4)).expect("source A");
    let mut session_a = new_session(&mut source_a);
    let action = session_a
        .request_start(capabilities(adapter_a), catalog_a, full_settings(), true)
        .expect("action A");

    let adapter_b = adapter_id(5);
    let (fake_b, handle_b) = FakeBridge::new(adapter_b, full_catalog(adapter_b));
    let mut source_b = BoundNativeAvBridge::new(fake_b, session_id(5)).expect("source B");
    assert!(matches!(
        action.execute_source(&mut session_a, &mut source_b),
        Err(AvCaptureError::OwnerMismatch)
    ));
    assert!(handle_b.lock().expect("fake state").operations.is_empty());
}

#[test]
fn buffer_ingress_requires_the_exact_session_generation_and_stream_epoch() {
    let (mut source, handle, mut session) = setup_started(6);
    let stamp = handle.lock().expect("fake state").stamps[0];
    let released = Arc::new(AtomicUsize::new(0));
    let buffer = NativeAvBuffer::new(
        stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(1_024, &released),
    )
    .expect("buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(buffer));
    let outcome = session
        .poll_source(&mut source)
        .expect("poll")
        .expect("event");
    assert_eq!(outcome.queue, Some(AvQueuePush::Accepted));
    let buffer = session
        .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(175_000_000))
        .expect("pop")
        .expect("queued buffer");
    assert_eq!(buffer.retained_bytes(), 1_024);
    assert_eq!(buffer.sequence(), 1);
    assert!(buffer.timestamp().is_some());
    buffer.release();
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn ingress_rejects_sequence_gaps_replays_and_source_pts_rollback_before_queueing() {
    let (mut source, handle, mut session) = setup_started(40);
    let stamp = session
        .source_stamp(AvSourceClass::Microphone)
        .expect("microphone stamp");
    let released = Arc::new(AtomicUsize::new(0));
    let gap = NativeAvBuffer::new(
        stamp,
        native_timing(2, 80_000_000, 10_000_000, 185_000_000),
        audio_format(48_000, 1),
        lease(16, &released),
    )
    .expect("gap buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(gap));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::OutOfOrderBuffer)
    ));

    let first = NativeAvBuffer::new(
        stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(16, &released),
    )
    .expect("first buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(first));
    session
        .poll_source(&mut source)
        .expect("first poll")
        .expect("first event");

    let replay = NativeAvBuffer::new(
        stamp,
        native_timing(1, 80_000_000, 10_000_000, 185_000_000),
        audio_format(48_000, 1),
        lease(16, &released),
    )
    .expect("replayed buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(replay));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::OutOfOrderBuffer)
    ));

    let rollback = NativeAvBuffer::new(
        stamp,
        native_timing(2, 69_000_000, 10_000_000, 185_000_000),
        audio_format(48_000, 1),
        lease(16, &released),
    )
    .expect("rollback buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(rollback));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::SourceTimestampRollback)
    ));

    let second = NativeAvBuffer::new(
        stamp,
        native_timing(2, 80_000_000, 10_000_000, 185_000_000),
        audio_format(48_000, 1),
        lease(16, &released),
    )
    .expect("second buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(second));
    session
        .poll_source(&mut source)
        .expect("second poll")
        .expect("second event");
    for _ in 0..2 {
        session
            .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(185_000_000))
            .expect("pop")
            .expect("queued buffer")
            .release();
    }
    assert_eq!(released.load(Ordering::SeqCst), 5);
}

#[test]
fn pause_resume_advances_epochs_and_rejects_delayed_callbacks() {
    let (mut source, handle, mut session) = setup_started(7);
    let old_stamp = handle.lock().expect("fake state").stamps[0];
    let pause = session.request_pause().expect("pause action");
    let AvActionExecution::Acknowledged(acknowledgement) = pause
        .execute_source(&mut session, &mut source)
        .expect("pause dispatch")
    else {
        panic!("pause must acknowledge");
    };
    session.complete(acknowledgement).expect("pause complete");
    assert_eq!(session.state(), AvSessionState::Paused);

    let resume = session
        .request_resume(capabilities(adapter_id(7)), full_catalog(adapter_id(7)))
        .expect("resume action");
    let AvActionExecution::Acknowledged(acknowledgement) = resume
        .execute_source(&mut session, &mut source)
        .expect("resume dispatch")
    else {
        panic!("resume must acknowledge");
    };
    let new_stamp = handle.lock().expect("fake state").stamps[0];
    assert!(new_stamp.stream_epoch().get() > old_stamp.stream_epoch().get());
    session.complete(acknowledgement).expect("resume complete");

    let released = Arc::new(AtomicUsize::new(0));
    let delayed = NativeAvBuffer::new(
        old_stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("delayed buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(delayed));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::StaleSourceStamp)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 1);

    let uncalibrated = NativeAvBuffer::new(
        new_stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("new epoch buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(uncalibrated));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::CalibrationRequired)
    ));
    session
        .calibrate_source(
            new_stamp,
            AvSyncPolicy::default(),
            &session_calibration_samples(),
        )
        .expect("new epoch calibration");
    let first = NativeAvBuffer::new(
        new_stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("first corrected buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(first));
    assert_eq!(
        session
            .poll_source(&mut source)
            .expect("poll corrected")
            .expect("event")
            .queue,
        Some(AvQueuePush::Accepted)
    );
    session
        .pop_buffer(AvSourceClass::Microphone, MonotonicTimeNs::new(175_000_000))
        .expect("pop")
        .expect("buffer")
        .release();
    assert_eq!(released.load(Ordering::SeqCst), 3);
}

#[test]
fn camera_preview_toggle_is_an_exact_reconfiguration() {
    let (mut source, _handle, mut session) = setup_started(8);
    assert!(session.camera_preview_enabled());
    let adapter = adapter_id(8);
    let action = session
        .request_reconfigure(
            capabilities(adapter),
            full_catalog(adapter),
            full_settings(),
            false,
        )
        .expect("reconfigure action");
    let AvActionExecution::Acknowledged(acknowledgement) = action
        .execute_source(&mut session, &mut source)
        .expect("reconfigure")
    else {
        panic!("reconfigure must acknowledge");
    };
    session
        .complete(acknowledgement)
        .expect("reconfigure complete");
    assert_eq!(session.state(), AvSessionState::Recording);
    assert!(!session.camera_preview_enabled());
}

#[test]
fn permission_revocation_disables_only_the_optional_source() {
    let (mut source, handle, mut session) = setup_started(9);
    let stamp = control_stamp(&source, 1, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp,
            class: AvSourceClass::Microphone,
            state: PermissionState::Revoked,
        });
    let outcome = session
        .poll_source(&mut source)
        .expect("poll")
        .expect("event");
    assert_eq!(outcome.disabled_sources, vec![AvSourceClass::Microphone]);
    assert_eq!(session.state(), AvSessionState::Recording);
    assert_eq!(outcome.diagnostics[0].code, AvStableCode::PermissionRevoked);
}

#[test]
fn permission_event_before_dispatch_invalidates_start_and_replay_cannot_reinstall_it() {
    let adapter = adapter_id(32);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(32)).expect("source");
    let mut session = new_session(&mut source);
    let stale_start = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start");
    let stamp = control_stamp(&source, 1, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp,
            class: AvSourceClass::Microphone,
            state: PermissionState::Granted,
        });
    let outcome = session
        .poll_source(&mut source)
        .expect("control event")
        .expect("event");
    assert!(outcome.native_reconfigure_required);
    assert_eq!(session.state(), AvSessionState::Idle);
    assert!(matches!(
        stale_start.execute_source(&mut session, &mut source),
        Err(AvCaptureError::StaleOperation)
    ));
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp,
            class: AvSourceClass::Microphone,
            state: PermissionState::Granted,
        });
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::StaleControlEvent)
    ));
    assert!(handle.lock().expect("fake state").operations.is_empty());
}

#[test]
fn every_catalog_change_reason_invalidates_a_dispatched_start_and_retains_teardown_stamps() {
    for (offset, reason) in [
        CatalogChangeReason::Hotplug,
        CatalogChangeReason::DefaultChanged,
        CatalogChangeReason::WirelessProfileChanged,
        CatalogChangeReason::CapabilityChanged,
    ]
    .into_iter()
    .enumerate()
    {
        let seed = 33 + u8::try_from(offset).expect("seed");
        let adapter = adapter_id(seed);
        let catalog = full_catalog(adapter);
        let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
        let mut source = BoundNativeAvBridge::new(fake, session_id(seed)).expect("source");
        let mut session = new_session(&mut source);
        let start = session
            .request_start(capabilities(adapter), catalog, full_settings(), true)
            .expect("start");
        let AvActionExecution::Acknowledged(held_ack) = start
            .execute_source(&mut session, &mut source)
            .expect("native start")
        else {
            panic!("start must acknowledge");
        };
        let changed = AvDeviceCatalog::new(adapter, 2, full_catalog(adapter).devices().to_vec())
            .expect("revision two catalog");
        let stamp = control_stamp(&source, 2, 1);
        handle
            .lock()
            .expect("fake state")
            .events
            .push_back(NativeAvEvent::CatalogChanged {
                stamp,
                catalog: changed,
                reason,
            });
        let outcome = session
            .poll_source(&mut source)
            .expect("catalog event")
            .expect("event");
        assert!(outcome.native_reconfigure_required);
        assert_eq!(session.state(), AvSessionState::TeardownRequired);
        assert!(matches!(
            session.complete(held_ack),
            Err(AvCaptureError::StaleOperation)
        ));
        let stop = session.request_stop().expect("stop").expect("stop action");
        let AvActionExecution::Acknowledged(stop_ack) = stop
            .execute_source(&mut session, &mut source)
            .expect("stop dispatch")
        else {
            panic!("stop must acknowledge");
        };
        assert_eq!(handle.lock().expect("fake state").stamps.len(), 3);
        session.complete(stop_ack).expect("stop complete");
    }
}

#[test]
fn permission_and_catalog_events_invalidate_dispatched_reconfigure_and_resume() {
    let (mut source, handle, mut session) = setup_started(37);
    let adapter = adapter_id(37);
    let reconfigure = session
        .request_reconfigure(
            capabilities(adapter),
            full_catalog(adapter),
            full_settings(),
            false,
        )
        .expect("reconfigure");
    let AvActionExecution::Acknowledged(held_reconfigure) = reconfigure
        .execute_source(&mut session, &mut source)
        .expect("native reconfigure")
    else {
        panic!("reconfigure must acknowledge");
    };
    let stamp = control_stamp(&source, 1, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp,
            class: AvSourceClass::Microphone,
            state: PermissionState::Granted,
        });
    session
        .poll_source(&mut source)
        .expect("permission event")
        .expect("event");
    assert_eq!(session.state(), AvSessionState::TeardownRequired);
    assert!(matches!(
        session.complete(held_reconfigure),
        Err(AvCaptureError::StaleOperation)
    ));

    let (mut source, handle, mut session) = setup_started(38);
    let pause = session.request_pause().expect("pause");
    let AvActionExecution::Acknowledged(pause_ack) = pause
        .execute_source(&mut session, &mut source)
        .expect("pause dispatch")
    else {
        panic!("pause must acknowledge");
    };
    session.complete(pause_ack).expect("pause complete");
    let adapter = adapter_id(38);
    let resume = session
        .request_resume(capabilities(adapter), full_catalog(adapter))
        .expect("resume");
    let AvActionExecution::Acknowledged(held_resume) = resume
        .execute_source(&mut session, &mut source)
        .expect("native resume")
    else {
        panic!("resume must acknowledge");
    };
    let stamp = control_stamp(&source, 2, 1);
    let changed = AvDeviceCatalog::new(adapter, 2, full_catalog(adapter).devices().to_vec())
        .expect("catalog revision");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::CatalogChanged {
            stamp,
            catalog: changed,
            reason: CatalogChangeReason::Hotplug,
        });
    session
        .poll_source(&mut source)
        .expect("catalog event")
        .expect("event");
    assert_eq!(session.state(), AvSessionState::TeardownRequired);
    assert!(matches!(
        session.complete(held_resume),
        Err(AvCaptureError::StaleOperation)
    ));
}

#[test]
fn control_event_revision_sequence_and_catalog_stamp_must_be_monotonic_and_exact() {
    let (mut source, handle, mut session) = setup_started(39);
    let first = control_stamp(&source, 2, 2);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp: first,
            class: AvSourceClass::Microphone,
            state: PermissionState::Granted,
        });
    session
        .poll_source(&mut source)
        .expect("first control event")
        .expect("event");

    let rollback = control_stamp(&source, 1, 3);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp: rollback,
            class: AvSourceClass::Microphone,
            state: PermissionState::Granted,
        });
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::StaleControlEvent)
    ));

    let mismatched = control_stamp(&source, 3, 4);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::CatalogChanged {
            stamp: mismatched,
            catalog: AvDeviceCatalog::new(
                adapter_id(39),
                2,
                full_catalog(adapter_id(39)).devices().to_vec(),
            )
            .expect("catalog"),
            reason: CatalogChangeReason::CapabilityChanged,
        });
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::InvalidControlEventStamp)
    ));
}

#[test]
fn unplug_and_wireless_profile_change_never_silently_rebind() {
    let (mut source, handle, mut session) = setup_started(10);
    let adapter = adapter_id(10);
    let changed_microphone = AvDeviceDescriptor::new(
        device_id(10),
        generation(2),
        AvSourceClass::Microphone,
        true,
        PermissionState::Granted,
        NativeRouteClass::WirelessTelephony,
        NativeTimestampKind::DeviceMonotonic,
        vec![audio_format(16_000, 1)],
    )
    .expect("changed microphone");
    let catalog = AvDeviceCatalog::new(
        adapter,
        2,
        vec![
            changed_microphone,
            device(
                11,
                AvSourceClass::SystemAudio,
                true,
                PermissionState::Granted,
                audio_format(48_000, 2),
            ),
            device(
                12,
                AvSourceClass::Camera,
                true,
                PermissionState::Granted,
                camera_format(1_920, 1_080, 30),
            ),
        ],
    )
    .expect("changed catalog");
    let stamp = control_stamp(&source, 2, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::CatalogChanged {
            stamp,
            catalog,
            reason: CatalogChangeReason::WirelessProfileChanged,
        });
    let outcome = session
        .poll_source(&mut source)
        .expect("poll")
        .expect("event");
    assert_eq!(outcome.disabled_sources, vec![AvSourceClass::Microphone]);
    assert_eq!(
        outcome.diagnostics[0].code,
        AvStableCode::FormatRenegotiationRequired
    );
    assert_eq!(
        outcome.diagnostics[0].capability,
        Some(AvCapabilityBucket::AudioStandard)
    );
    assert_eq!(session.state(), AvSessionState::Recording);
}

#[test]
fn sleep_drains_ingress_and_requires_an_epoch_advancing_resume() {
    let (mut source, handle, mut session) = setup_started(11);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Sleep);
    let outcome = session
        .poll_source(&mut source)
        .expect("sleep poll")
        .expect("sleep event");
    assert_eq!(session.state(), AvSessionState::Suspended);
    assert_eq!(outcome.diagnostics[0].code, AvStableCode::Sleep);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Wake);
    let outcome = session
        .poll_source(&mut source)
        .expect("wake poll")
        .expect("wake event");
    assert_eq!(outcome.diagnostics[0].code, AvStableCode::Wake);
    let resume = session
        .request_resume(capabilities(adapter_id(11)), full_catalog(adapter_id(11)))
        .expect("resume after wake");
    let AvActionExecution::Acknowledged(acknowledgement) = resume
        .execute_source(&mut session, &mut source)
        .expect("resume dispatch")
    else {
        panic!("resume must acknowledge");
    };
    session.complete(acknowledgement).expect("resume complete");
    assert_eq!(session.state(), AvSessionState::Recording);
}

#[test]
fn failed_stop_remains_retryable_until_terminal_teardown_is_confirmed() {
    let (mut source, handle, mut session) = setup_started(12);
    handle.lock().expect("fake state").fail_next = Some(NativeAvFailure {
        code: NativeAvFailureCode::Timeout,
        retryable: true,
    });
    let stop = session.request_stop().expect("stop").expect("stop action");
    let AvActionExecution::Failed(failure) = stop
        .execute_source(&mut session, &mut source)
        .expect("bound failure")
    else {
        panic!("first stop must fail");
    };
    let native = session.complete_failure(failure).expect("complete failure");
    assert!(native.retryable);
    assert_eq!(session.state(), AvSessionState::TeardownRequired);
    let retry = session
        .request_stop()
        .expect("retry stop")
        .expect("retry action");
    let AvActionExecution::Acknowledged(acknowledgement) = retry
        .execute_source(&mut session, &mut source)
        .expect("retry dispatch")
    else {
        panic!("retry must acknowledge");
    };
    session.complete(acknowledgement).expect("stop complete");
    assert_eq!(session.state(), AvSessionState::Stopped);
    let result = session.terminal_result().expect("terminal result");
    assert!(session.request_stop().expect("idempotent stop").is_none());
    assert_eq!(session.terminal_result(), Some(result));
    let state = handle.lock().expect("fake state");
    assert_eq!(state.native_release_count, 1);
    assert_eq!(state.reconciliations, 2);
}

#[test]
fn lost_teardown_ack_can_be_reissued_without_reopening_capture() {
    let (mut source, handle, mut session) = setup_started(28);
    let first = session.request_stop().expect("stop").expect("stop action");
    let AvActionExecution::Acknowledged(delayed_first_ack) = first
        .execute_source(&mut session, &mut source)
        .expect("first stop dispatch")
    else {
        panic!("first stop must acknowledge");
    };
    assert_eq!(session.state(), AvSessionState::Stopping);
    let retry = session.retry_teardown().expect("retry lost stop");
    let AvActionExecution::Acknowledged(acknowledgement) = retry
        .execute_source(&mut session, &mut source)
        .expect("retry dispatch")
    else {
        panic!("retry must acknowledge");
    };
    session.complete(acknowledgement).expect("retry complete");
    assert_eq!(session.state(), AvSessionState::Stopped);
    assert!(matches!(
        session.complete(delayed_first_ack),
        Err(AvCaptureError::StaleOperation)
    ));
    let terminal_result = session.terminal_result().expect("stable terminal result");
    assert_eq!(terminal_result.kind, AvOperationKind::Stop);
    let state = handle.lock().expect("fake state");
    assert_eq!(
        state.operations,
        vec![AvOperationKind::Start, AvOperationKind::Stop]
    );
    assert_eq!(state.reconciliations, 2);
    assert_eq!(state.native_release_count, 1);
    assert_eq!(
        state.last_native_timeout,
        Some(std::time::Duration::from_secs(10))
    );
    drop(state);
    assert!(session.request_stop().expect("repeat stop").is_none());
    assert_eq!(session.terminal_result(), Some(terminal_result));
}

#[test]
fn ingress_backpressure_is_bounded_nonblocking_and_releases_every_lease() {
    let (mut source, handle, mut session) = setup_started(13);
    let camera_stamp = handle
        .lock()
        .expect("fake state")
        .stamps
        .iter()
        .copied()
        .find(|stamp| stamp.class() == AvSourceClass::Camera)
        .expect("camera stamp");
    let queue_spec = session
        .source_ingress_queue_spec(AvSourceClass::Camera)
        .expect("camera session ingress partition");
    const RETAINED_BYTES_PER_BUFFER: u64 = 20 * 1024 * 1024;
    let queue_capacity = usize::from(queue_spec.max_buffers).min(
        usize::try_from(queue_spec.max_bytes / RETAINED_BYTES_PER_BUFFER)
            .expect("camera queue byte capacity fits usize"),
    );
    let released = Arc::new(AtomicUsize::new(0));
    let mut drops = 0_usize;
    for index in 0..9_u64 {
        let buffer = NativeAvBuffer::new(
            camera_stamp,
            native_timing(
                index + 1,
                70_000_000 + index * 33_000_000,
                33_000_000,
                175_000_000 + index * 33_000_000,
            ),
            camera_format(1_920, 1_080, 30),
            opaque_lease(RETAINED_BYTES_PER_BUFFER, index, &released),
        )
        .expect("buffer");
        handle
            .lock()
            .expect("fake state")
            .events
            .push_back(NativeAvEvent::Buffer(buffer));
        let outcome = session
            .poll_source(&mut source)
            .expect("poll")
            .expect("buffer event");
        if matches!(outcome.queue, Some(AvQueuePush::DroppedOldest { .. })) {
            drops += 1;
        }
    }
    let expected_drops = 9_usize - queue_capacity;
    assert_eq!(drops, expected_drops);
    assert_eq!(released.load(Ordering::SeqCst), expected_drops);
    let mut popped = 0;
    while let Some(buffer) = session
        .pop_buffer(AvSourceClass::Camera, MonotonicTimeNs::new(439_000_000))
        .expect("pop")
    {
        popped += 1;
        buffer.release();
    }
    assert_eq!(popped, queue_capacity);
    assert_eq!(released.load(Ordering::SeqCst), 9);
}

#[test]
fn an_uncorrected_buffer_cannot_be_injected_directly_into_ingress() {
    let (_source, handle, _session) = setup_started(14);
    let stamp = handle.lock().expect("fake state").stamps[0];
    let released = Arc::new(AtomicUsize::new(0));
    let mut queue = AvIngressQueue::new(
        stamp,
        audio_format(48_000, 1),
        AvQueueSpec {
            max_buffers: 1,
            max_bytes: 64,
            max_age_ns: 1_000,
            backpressure: AvBackpressurePolicy::DropNewest,
            producer_blocks: false,
        },
    )
    .expect("queue");
    let buffer = NativeAvBuffer::new(
        stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("buffer");
    assert!(matches!(
        queue.push(buffer),
        Err(AvCaptureError::UncorrectedBuffer)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn consumer_poll_expires_old_buffers_even_when_the_producer_is_idle() {
    let (mut source, handle, mut session) = setup_started(27);
    let stamp = handle.lock().expect("fake state").stamps[0];
    let released = Arc::new(AtomicUsize::new(0));
    let first = NativeAvBuffer::new(
        stamp,
        native_timing(1, 70_000_000, 10_000_000, 175_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(first));
    session
        .poll_source(&mut source)
        .expect("poll")
        .expect("buffer event");
    assert!(
        session
            .pop_buffer(
                AvSourceClass::Microphone,
                MonotonicTimeNs::new(2_175_000_001),
            )
            .expect("consumer poll")
            .is_none()
    );
    assert_eq!(released.load(Ordering::SeqCst), 1);
    let stale = NativeAvBuffer::new(
        stamp,
        native_timing(2, 95_000_000, 10_000_000, 200_000_000),
        audio_format(48_000, 1),
        lease(32, &released),
    )
    .expect("stale-clock buffer");
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(stale));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::NonMonotonicMasterClock)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

#[test]
fn gain_mute_mix_and_clipping_preserve_the_declared_audio_timeline() {
    let mut mixer = AudioMixEngine::new(48_000, 2, ClippingPolicy::HardLimit).expect("mixer");
    mixer
        .set_source(
            AvSourceClass::Microphone,
            AudioSourceMixSettings {
                gain_milli: 1_000,
                muted: false,
                ramp_frames: 0,
            },
        )
        .expect("microphone gain");
    mixer
        .set_source(
            AvSourceClass::SystemAudio,
            AudioSourceMixSettings {
                gain_milli: 1_000,
                muted: false,
                ramp_frames: 0,
            },
        )
        .expect("system gain");
    let microphone = vec![0.75; 960];
    let system = vec![0.75; 960];
    let mixed = mixer
        .mix(0, 480, Some(&microphone), Some(&system))
        .expect("mix");
    assert_eq!(mixed.timestamp.duration_ns, 10_000_000);
    assert!(
        mixed
            .samples_for_local_pipeline()
            .iter()
            .all(|sample| *sample == 1.0)
    );
    assert!(mixed.output_clipped);
    assert!(mixed.meters.iter().all(|meter| !meter.clipped));
    mixer
        .set_source(
            AvSourceClass::Microphone,
            AudioSourceMixSettings {
                gain_milli: 1_000,
                muted: true,
                ramp_frames: 480,
            },
        )
        .expect("mute ramp");
    let ramped = mixer
        .mix(10_000_000, 480, Some(&microphone), None)
        .expect("ramped mute");
    let samples = ramped.samples_for_local_pipeline();
    assert!(samples[0] > samples[samples.len() - 1]);
    let silent = mixer
        .mix(20_000_000, 480, Some(&microphone), None)
        .expect("muted block");
    assert!(
        silent
            .samples_for_local_pipeline()
            .iter()
            .all(|sample| sample.abs() < 0.000_01)
    );
    assert!(matches!(
        mixer.mix(40_000_000, 480, None, None),
        Err(AvCaptureError::AudioTimelineDiscontinuity)
    ));
    assert!(matches!(
        mixer.mark_discontinuity(1),
        Err(AvCaptureError::AudioTimelineDiscontinuity)
    ));
    mixer
        .mark_discontinuity(40_000_000)
        .expect("forward discontinuity");
    let discontinuous = mixer
        .mix(40_000_000, 480, None, None)
        .expect("discontinuous block");
    assert!(discontinuous.timestamp.discontinuity);
}

#[test]
fn mixer_rejects_non_finite_or_misaligned_media_and_soft_limits_valid_media() {
    let mut mixer = AudioMixEngine::new(48_000, 1, ClippingPolicy::SoftLimit).expect("mixer");
    assert!(matches!(
        mixer.mix(0, 2, Some(&[f32::NAN, 0.0]), None),
        Err(AvCaptureError::InvalidAudioBlock)
    ));
    assert!(matches!(
        mixer.mix(0, 2, Some(&[0.0]), None),
        Err(AvCaptureError::InvalidAudioBlock)
    ));
    mixer
        .set_source(
            AvSourceClass::Microphone,
            AudioSourceMixSettings {
                gain_milli: 4_000,
                muted: false,
                ramp_frames: 0,
            },
        )
        .expect("gain");
    let output = mixer
        .mix(0, 2, Some(&[1.0, -1.0]), None)
        .expect("soft-limited mix");
    assert!(
        output
            .samples_for_local_pipeline()
            .iter()
            .all(|sample| sample.abs() < 1.0)
    );
    assert!(output.meters[0].clipped);
}

#[test]
fn rational_mixer_accumulator_has_zero_partition_drift_for_sixty_minutes_at_common_rates() {
    for sample_rate in [44_100_u32, 48_000, 96_000] {
        let mut mixer =
            AudioMixEngine::new(sample_rate, 1, ClippingPolicy::HardLimit).expect("mixer");
        mixer
            .set_source(
                AvSourceClass::Microphone,
                AudioSourceMixSettings {
                    gain_milli: 1_250,
                    muted: false,
                    ramp_frames: sample_rate / 2,
                },
            )
            .expect("gain ramp");
        let total_frames = u128::from(sample_rate) * 3_600;
        let partitions = [
            1_u32,
            7,
            1_023,
            sample_rate - 1_031,
            sample_rate.saturating_mul(17).saturating_add(3),
        ];
        let mut completed = 0_u128;
        let mut segment_origin = 9_000_000_u64;
        let mut segment_frames = 0_u128;
        let mut partition = 0_usize;
        let mut discontinuity_inserted = false;
        while completed < total_frames {
            if !discontinuity_inserted && completed >= total_frames / 2 {
                let current = segment_origin
                    + u64::try_from(segment_frames * 1_000_000_000 / u128::from(sample_rate))
                        .expect("timeline range");
                segment_origin = current + 49_999_999;
                segment_frames = 0;
                mixer
                    .mark_discontinuity(segment_origin)
                    .expect("bounded discontinuity");
                mixer
                    .set_source(
                        AvSourceClass::SystemAudio,
                        AudioSourceMixSettings {
                            gain_milli: 0,
                            muted: true,
                            ramp_frames: sample_rate,
                        },
                    )
                    .expect("mute ramp");
                discontinuity_inserted = true;
            }
            let remaining = total_frames - completed;
            let frames = u32::try_from(remaining.min(u128::from(partitions[partition])))
                .expect("partition range");
            let expected_pts = segment_origin
                + u64::try_from(segment_frames * 1_000_000_000 / u128::from(sample_rate))
                    .expect("expected timestamp");
            let timestamp = mixer
                .advance_silence_timeline(expected_pts, frames)
                .expect("rational advance");
            segment_frames += u128::from(frames);
            completed += u128::from(frames);
            let expected_end = segment_origin
                + u64::try_from(segment_frames * 1_000_000_000 / u128::from(sample_rate))
                    .expect("expected end");
            assert_eq!(timestamp.pts_ns, expected_pts);
            assert_eq!(timestamp.end_ns(), expected_end);
            assert!(timestamp.duration_ns > 0);
            if timestamp.discontinuity {
                assert!(discontinuity_inserted);
            }
            partition = (partition + 1) % partitions.len();
        }
        assert_eq!(completed, total_frames);
    }
}

#[test]
fn ui_events_are_throttled_coalesced_and_structurally_private() {
    let mut events = AvUiEventCoalescer::new(DEFAULT_UI_EVENT_INTERVAL_NS).expect("coalescer");
    for peak in [MeterBucket::Low, MeterBucket::Medium, MeterBucket::High] {
        events
            .push(
                MonotonicTimeNs::new(1),
                AvUiEvent::Meter(AudioMeterSummary {
                    class: AvSourceClass::Microphone,
                    rms: peak,
                    peak,
                    clipped: false,
                }),
            )
            .expect("push meter");
    }
    events
        .push(
            MonotonicTimeNs::new(1),
            AvUiEvent::CameraPreview { enabled: true },
        )
        .expect("push preview");
    let first = events
        .drain_ready(MonotonicTimeNs::new(1))
        .expect("first drain");
    assert_eq!(first.len(), 2);
    assert!(first.contains(&AvUiEvent::Meter(AudioMeterSummary {
        class: AvSourceClass::Microphone,
        rms: MeterBucket::High,
        peak: MeterBucket::High,
        clipped: false,
    })));
    let debug = format!("{first:?}");
    assert!(!debug.contains("device"));
    assert!(!debug.contains("label"));
    assert!(!debug.contains("media"));
    events
        .push(MonotonicTimeNs::new(2), AvUiEvent::Paused)
        .expect("push pause");
    assert!(
        events
            .drain_ready(MonotonicTimeNs::new(DEFAULT_UI_EVENT_INTERVAL_NS - 1))
            .expect("throttled drain")
            .is_empty()
    );
    assert_eq!(
        events
            .drain_ready(MonotonicTimeNs::new(DEFAULT_UI_EVENT_INTERVAL_NS + 1))
            .expect("ready drain"),
        vec![AvUiEvent::Paused]
    );
}

fn calibration_samples(base_ns: u64, latency_ns: u64, drift_ppm: i64) -> Vec<CalibrationSample> {
    (0..7_u64)
        .map(|index| {
            let master_elapsed = index * 10_000_000;
            let source_elapsed = ((i128::from(master_elapsed) * i128::from(1_000_000 + drift_ppm))
                / 1_000_000) as u64;
            let jitter = match index % 3 {
                0 => 500_000,
                1 => 0,
                _ => 250_000,
            };
            CalibrationSample {
                master_arrival: MonotonicTimeNs::new(
                    base_ns + master_elapsed + latency_ns + jitter,
                ),
                source_pts_ns: source_elapsed,
                latency: SourceLatency {
                    reported_ns: latency_ns,
                    confidence: LatencyConfidence::Measured,
                },
            }
        })
        .collect()
}

#[test]
fn startup_calibration_reports_confidence_and_enforces_the_80ms_budget() {
    let samples = calibration_samples(40_000_000, 12_000_000, 100);
    let calibration = StartupCalibration::measure(&samples).expect("calibration");
    assert_eq!(calibration.sample_count, 7);
    assert!(calibration.spread_ns <= 600_000);
    assert_eq!(calibration.confidence, CalibrationConfidence::High);
    SourceTimebase::new(AvSyncPolicy::default(), calibration, samples[3])
        .expect("timebase in budget");

    let bad = StartupCalibration {
        offset_ns: 200_000_000,
        ..calibration
    };
    assert!(matches!(
        SourceTimebase::new(AvSyncPolicy::default(), bad, samples[3]),
        Err(AvCaptureError::StartupOffsetBudgetExceeded)
    ));
}

#[test]
fn deterministic_60_minute_drift_runs_stay_inside_the_50ms_charter() {
    for drift_ppm in [-5_000_i64, -800, -250, 0, 175, 800, 5_000] {
        let base_ns = 40_000_000;
        let latency_ns = 12_000_000;
        let samples = calibration_samples(base_ns, latency_ns, drift_ppm);
        let calibration = StartupCalibration::measure(&samples).expect("calibration");
        let anchor = samples[6];
        let mut timebase =
            SourceTimebase::new(AvSyncPolicy::default(), calibration, anchor).expect("timebase");
        let mut final_offset = 0_i64;
        for second in 1..=3_600_u64 {
            let master_elapsed = 60_000_000 + second * 1_000_000_000;
            let source_pts = ((i128::from(master_elapsed) * i128::from(1_000_000 + drift_ppm))
                / 1_000_000) as u64;
            let jitter = match second % 5 {
                0 => 800_000,
                1 => 300_000,
                2 => 0,
                3 => 600_000,
                _ => 100_000,
            };
            let corrected = timebase
                .observe(
                    source_pts,
                    1_000_000_000,
                    MonotonicTimeNs::new(base_ns + master_elapsed + latency_ns + jitter),
                    SourceLatency {
                        reported_ns: latency_ns,
                        confidence: LatencyConfidence::Measured,
                    },
                    false,
                )
                .expect("drift observation");
            final_offset = corrected.observed_offset_ns;
            assert!(corrected.frame.end_ns() > corrected.frame.pts_ns);
        }
        assert!(
            final_offset.unsigned_abs() <= LONG_SYNC_BUDGET_NS,
            "drift {drift_ppm} ppm ended at {final_offset}ns"
        );
    }
}

#[test]
fn sync_policy_cannot_admit_more_drift_than_its_correction_rate_can_repair() {
    let insufficient = AvSyncPolicy {
        max_abs_drift_ppm: 5_000,
        max_correction_ns_per_second: 4_999_999,
        ..AvSyncPolicy::default()
    };
    assert!(matches!(
        insufficient.validate(),
        Err(AvCaptureError::InvalidSyncPolicyV2)
    ));
    assert!(AvSyncPolicy::default().validate().is_ok());
}

#[test]
fn long_budget_boundary_is_enforced_and_latency_confidence_changes_need_discontinuity() {
    let samples = session_calibration_samples();
    let calibration = StartupCalibration::measure(&samples).expect("calibration");
    let mut near =
        SourceTimebase::new(AvSyncPolicy::default(), calibration, samples[6]).expect("timebase");
    near.observe(
        70_000_000,
        59_000_000,
        MonotonicTimeNs::new(175_000_000),
        SourceLatency {
            reported_ns: 5_000_000,
            confidence: LatencyConfidence::Measured,
        },
        false,
    )
    .expect("first long frame");
    let boundary = near
        .observe(
            80_000_000,
            10_000_000,
            MonotonicTimeNs::new(185_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            false,
        )
        .expect("49ms jitter remains in budget");
    assert_eq!(boundary.observed_offset_ns.unsigned_abs(), 49_000_000);

    let mut over =
        SourceTimebase::new(AvSyncPolicy::default(), calibration, samples[6]).expect("timebase");
    over.observe(
        70_000_000,
        61_000_001,
        MonotonicTimeNs::new(175_000_000),
        SourceLatency {
            reported_ns: 5_000_000,
            confidence: LatencyConfidence::Measured,
        },
        false,
    )
    .expect("first overlong frame");
    assert!(matches!(
        over.observe(
            80_000_000,
            10_000_000,
            MonotonicTimeNs::new(185_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            false,
        ),
        Err(AvCaptureError::SynchronizationBudgetExceeded)
    ));
    let recovered = over
        .observe(
            80_000_000,
            10_000_000,
            MonotonicTimeNs::new(185_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            true,
        )
        .expect("declared discontinuity recovers");
    assert!(recovered.frame.discontinuity);

    let mut confidence =
        SourceTimebase::new(AvSyncPolicy::default(), calibration, samples[6]).expect("timebase");
    confidence
        .observe(
            70_000_000,
            10_000_000,
            MonotonicTimeNs::new(175_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            false,
        )
        .expect("first frame");
    assert!(matches!(
        confidence.observe(
            80_000_000,
            10_000_000,
            MonotonicTimeNs::new(185_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Reported,
            },
            false,
        ),
        Err(AvCaptureError::ClockDiscontinuityRequired)
    ));
    assert!(
        confidence
            .observe(
                80_000_000,
                10_000_000,
                MonotonicTimeNs::new(185_000_000),
                SourceLatency {
                    reported_ns: 5_000_000,
                    confidence: LatencyConfidence::Reported,
                },
                true,
            )
            .expect("confidence discontinuity")
            .frame
            .discontinuity
    );
}

#[test]
fn timebase_extreme_values_fail_with_a_bounded_error_instead_of_saturating() {
    let calibration = StartupCalibration {
        offset_ns: 1,
        spread_ns: 0,
        sample_count: 7,
        confidence: CalibrationConfidence::High,
    };
    let mut timebase = SourceTimebase::new(
        AvSyncPolicy::default(),
        calibration,
        CalibrationSample {
            master_arrival: MonotonicTimeNs::new(u64::MAX),
            source_pts_ns: u64::MAX - 1,
            latency: SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Measured,
            },
        },
    )
    .expect("edge timebase");
    assert!(matches!(
        timebase.observe(
            u64::MAX - 2,
            2,
            MonotonicTimeNs::new(u64::MAX),
            SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Measured,
            },
            true,
        ),
        Err(AvCaptureError::InvalidFrameTimestamp)
    ));
}

#[test]
fn discontinuity_pause_and_resume_never_roll_the_output_timeline_back() {
    let samples = calibration_samples(20_000_000, 5_000_000, 200);
    let calibration = StartupCalibration::measure(&samples).expect("calibration");
    let mut timebase =
        SourceTimebase::new(AvSyncPolicy::default(), calibration, samples[6]).expect("timebase");
    let first = timebase
        .observe(
            1_000_200_000,
            10_000_000,
            MonotonicTimeNs::new(1_025_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            false,
        )
        .expect("first");
    timebase
        .pause(MonotonicTimeNs::new(1_030_000_000))
        .expect("pause");
    assert!(matches!(
        timebase.observe(
            2,
            10,
            MonotonicTimeNs::new(1_040_000_000),
            SourceLatency {
                reported_ns: 0,
                confidence: LatencyConfidence::Unknown,
            },
            true,
        ),
        Err(AvCaptureError::TimebasePaused)
    ));
    timebase
        .resume(MonotonicTimeNs::new(2_000_000_000))
        .expect("resume");
    let rebased = timebase
        .observe(
            0,
            10_000_000,
            MonotonicTimeNs::new(2_005_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            true,
        )
        .expect("rebased");
    assert!(rebased.frame.discontinuity);
    assert!(rebased.frame.pts_ns >= first.frame.end_ns());
    assert!(matches!(
        timebase.observe(
            0,
            10,
            MonotonicTimeNs::new(2_010_000_000),
            SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            false,
        ),
        Err(AvCaptureError::SourceTimestampRollback)
    ));
}

#[test]
fn foreign_owned_event_cannot_mutate_another_session() {
    let (_source_a, _handle_a, mut session_a) = setup_started(15);
    let (mut source_b, handle_b, _session_b) = setup_started(16);
    handle_b
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Wake);
    let event = source_b
        .poll_owned()
        .expect("poll B")
        .expect("owned event B");
    assert!(matches!(
        session_a.apply_owned_event(event),
        Err(AvCaptureError::OwnerMismatch)
    ));
    assert_eq!(session_a.state(), AvSessionState::Recording);
}

#[test]
fn stop_after_native_start_ack_quiesces_the_unapplied_streams() {
    let adapter = adapter_id(17);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(17)).expect("source");
    let mut session = new_session(&mut source);
    let start = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start");
    let AvActionExecution::Acknowledged(delayed_start_ack) = start
        .execute_source(&mut session, &mut source)
        .expect("native start")
    else {
        panic!("start must acknowledge");
    };
    let stop = session.request_stop().expect("stop").expect("stop action");
    let AvActionExecution::Acknowledged(stop_ack) = stop
        .execute_source(&mut session, &mut source)
        .expect("native stop")
    else {
        panic!("stop must acknowledge");
    };
    assert_eq!(handle.lock().expect("fake state").stamps.len(), 3);
    session.complete(stop_ack).expect("stop complete");
    assert_eq!(session.state(), AvSessionState::Stopped);
    assert!(matches!(
        session.complete(delayed_start_ack),
        Err(AvCaptureError::StaleOperation)
    ));
    assert_eq!(
        handle.lock().expect("fake state").operations,
        vec![AvOperationKind::Start, AvOperationKind::Stop]
    );
}

#[test]
fn permission_prompt_is_owner_bound_and_returns_to_idle() {
    let adapter = adapter_id(18);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog);
    let mut source = BoundNativeAvBridge::new(fake, session_id(18)).expect("source");
    let mut session = new_session(&mut source);
    let prompt = session
        .request_permission(AvSourceClass::Microphone)
        .expect("permission action");
    let AvActionExecution::Acknowledged(acknowledgement) = prompt
        .execute_source(&mut session, &mut source)
        .expect("permission dispatch")
    else {
        panic!("permission must acknowledge");
    };
    session
        .complete(acknowledgement)
        .expect("permission completion");
    assert_eq!(session.state(), AvSessionState::Idle);
    assert_eq!(
        handle.lock().expect("fake state").operations,
        vec![AvOperationKind::RequestPermission(
            AvSourceClass::Microphone
        )]
    );
}

#[test]
fn default_switch_event_requires_confirmation_and_native_reconfiguration() {
    let adapter = adapter_id(19);
    let original = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, original.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(19)).expect("source");
    let mut settings = full_settings();
    settings.microphone = DeviceSelectionV2::FollowDefault {
        format: audio_format(48_000, 1),
        allow_default_changes: false,
        confirmed_id: Some(device_id(10)),
    };
    let mut session = new_session(&mut source);
    let start = session
        .request_start(capabilities(adapter), original, settings, true)
        .expect("start");
    let AvActionExecution::Acknowledged(acknowledgement) = start
        .execute_source(&mut session, &mut source)
        .expect("start dispatch")
    else {
        panic!("start must acknowledge");
    };
    session.complete(acknowledgement).expect("start complete");
    let changed = AvDeviceCatalog::new(
        adapter,
        2,
        vec![
            device(
                10,
                AvSourceClass::Microphone,
                false,
                PermissionState::Granted,
                audio_format(48_000, 1),
            ),
            device(
                99,
                AvSourceClass::Microphone,
                true,
                PermissionState::Granted,
                audio_format(48_000, 1),
            ),
            device(
                11,
                AvSourceClass::SystemAudio,
                true,
                PermissionState::Granted,
                audio_format(48_000, 2),
            ),
            device(
                12,
                AvSourceClass::Camera,
                true,
                PermissionState::Granted,
                camera_format(1_920, 1_080, 30),
            ),
        ],
    )
    .expect("changed catalog");
    let stamp = control_stamp(&source, 2, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::CatalogChanged {
            stamp,
            catalog: changed,
            reason: CatalogChangeReason::DefaultChanged,
        });
    let outcome = session
        .poll_source(&mut source)
        .expect("poll")
        .expect("event");
    assert_eq!(outcome.disabled_sources, vec![AvSourceClass::Microphone]);
    assert!(outcome.native_reconfigure_required);
    assert_eq!(
        outcome.diagnostics[0].code,
        AvStableCode::DefaultConfirmationRequired
    );
}

#[test]
fn invalid_native_buffers_release_their_lease_on_every_constructor_error() {
    let (_source, handle, _session) = setup_started(20);
    let stamp = handle.lock().expect("fake state").stamps[0];
    let released = Arc::new(AtomicUsize::new(0));
    assert!(matches!(
        NativeAvBuffer::new(
            stamp,
            NativeAvBufferTiming {
                sequence: 1,
                source_pts_ns: 0,
                duration_ns: 0,
                arrival: MonotonicTimeNs::new(0),
                latency: SourceLatency {
                    reported_ns: 0,
                    confidence: LatencyConfidence::Unknown,
                },
                discontinuity: false,
            },
            audio_format(48_000, 1),
            lease(32, &released),
        ),
        Err(AvCaptureError::InvalidNativeBufferTiming)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert!(matches!(
        NativeAvBuffer::new(
            stamp,
            native_timing(1, 70_000_000, 1, 175_000_000),
            camera_format(640, 480, 30),
            lease(32, &released),
        ),
        Err(AvCaptureError::FormatClassMismatch)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

#[test]
fn disabled_and_readded_source_never_reuses_a_session_stream_epoch() {
    let (mut source, handle, mut session) = setup_started(21);
    let old_microphone = handle
        .lock()
        .expect("fake state")
        .stamps
        .iter()
        .copied()
        .find(|stamp| stamp.class() == AvSourceClass::Microphone)
        .expect("old microphone");
    let stamp = control_stamp(&source, 1, 1);
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::PermissionChanged {
            stamp,
            class: AvSourceClass::Microphone,
            state: PermissionState::Revoked,
        });
    session
        .poll_source(&mut source)
        .expect("poll revocation")
        .expect("revocation event");
    let adapter = adapter_id(21);
    let reconfigure = session
        .request_reconfigure(
            capabilities(adapter),
            full_catalog(adapter),
            full_settings(),
            true,
        )
        .expect("reconfigure");
    let AvActionExecution::Acknowledged(acknowledgement) = reconfigure
        .execute_source(&mut session, &mut source)
        .expect("reconfigure dispatch")
    else {
        panic!("reconfigure must acknowledge");
    };
    let state = handle.lock().expect("fake state");
    let new_microphone = state
        .stamps
        .iter()
        .copied()
        .find(|stamp| stamp.class() == AvSourceClass::Microphone)
        .expect("new microphone");
    assert!(new_microphone.stream_epoch().get() > old_microphone.stream_epoch().get());
    assert!(state.predecessor_stamps.contains(&old_microphone));
    drop(state);
    session
        .complete(acknowledgement)
        .expect("reconfigure complete");

    let released = Arc::new(AtomicUsize::new(0));
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(
            NativeAvBuffer::new(
                old_microphone,
                native_timing(1, 70_000_000, 10_000_000, 175_000_000),
                audio_format(48_000, 1),
                lease(16, &released),
            )
            .expect("delayed buffer"),
        ));
    assert!(matches!(
        session.poll_source(&mut source),
        Err(AvCaptureError::StaleSourceStamp)
    ));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn ambiguous_reconfigure_failure_is_a_predecessor_of_the_retry() {
    let (mut source, handle, mut session) = setup_started(22);
    handle.lock().expect("fake state").fail_next = Some(NativeAvFailure {
        code: NativeAvFailureCode::Timeout,
        retryable: true,
    });
    let adapter = adapter_id(22);
    let first = session
        .request_reconfigure(
            capabilities(adapter),
            full_catalog(adapter),
            full_settings(),
            false,
        )
        .expect("first reconfigure");
    let AvActionExecution::Failed(failure) = first
        .execute_source(&mut session, &mut source)
        .expect("ambiguous failure")
    else {
        panic!("first reconfigure must fail");
    };
    let ambiguous_stamps = handle.lock().expect("fake state").stamps.clone();
    session
        .complete_failure(failure)
        .expect("complete ambiguous failure");
    let retry = session
        .request_reconfigure(
            capabilities(adapter),
            full_catalog(adapter),
            full_settings(),
            false,
        )
        .expect("retry reconfigure");
    let AvActionExecution::Acknowledged(acknowledgement) = retry
        .execute_source(&mut session, &mut source)
        .expect("retry dispatch")
    else {
        panic!("retry must acknowledge");
    };
    let predecessors = &handle.lock().expect("fake state").predecessor_stamps;
    assert!(
        ambiguous_stamps
            .iter()
            .all(|stamp| predecessors.contains(stamp))
    );
    session.complete(acknowledgement).expect("retry complete");
    assert_eq!(session.state(), AvSessionState::Recording);
}

#[test]
fn resume_revalidates_permissions_generation_and_exact_formats() {
    let (mut source, _handle, mut session) = setup_started(23);
    let pause = session.request_pause().expect("pause");
    let AvActionExecution::Acknowledged(acknowledgement) = pause
        .execute_source(&mut session, &mut source)
        .expect("pause dispatch")
    else {
        panic!("pause must acknowledge");
    };
    session.complete(acknowledgement).expect("pause complete");
    let adapter = adapter_id(23);
    let missing_microphone = AvDeviceCatalog::new(
        adapter,
        2,
        full_catalog(adapter)
            .devices()
            .iter()
            .filter(|device| device.class() != AvSourceClass::Microphone)
            .cloned()
            .collect(),
    )
    .expect("catalog without microphone");
    assert!(matches!(
        session.request_resume(capabilities(adapter), missing_microphone),
        Err(AvCaptureError::ResumeRequiresReconfiguration)
    ));
    assert_eq!(session.state(), AvSessionState::Paused);
}

#[test]
fn calibration_rejects_nonmonotonic_source_samples() {
    let mut samples = calibration_samples(20_000_000, 5_000_000, 0);
    samples[4].source_pts_ns = samples[3].source_pts_ns;
    assert!(matches!(
        StartupCalibration::measure(&samples),
        Err(AvCaptureError::SourceTimestampRollback)
    ));
}

#[test]
fn live_snapshot_adapter_failure_returns_an_owner_bound_completion() {
    let adapter = adapter_id(24);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(24)).expect("source");
    let mut session = new_session(&mut source);
    let action = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start");
    handle.lock().expect("fake state").fail_capabilities = Some(NativeAvFailure {
        code: NativeAvFailureCode::BackendFault,
        retryable: true,
    });
    let AvActionExecution::Failed(failure) = action
        .execute_source(&mut session, &mut source)
        .expect("bound snapshot failure")
    else {
        panic!("snapshot call must fail");
    };
    let failure = session.complete_failure(failure).expect("complete failure");
    assert_eq!(failure.code, NativeAvFailureCode::BackendFault);
    assert_eq!(session.state(), AvSessionState::Idle);
    assert!(handle.lock().expect("fake state").operations.is_empty());
}

#[test]
fn sleep_during_ambiguous_start_requires_teardown_before_restart() {
    let adapter = adapter_id(25);
    let catalog = full_catalog(adapter);
    let (fake, handle) = FakeBridge::new(adapter, catalog.clone());
    let mut source = BoundNativeAvBridge::new(fake, session_id(25)).expect("source");
    let mut session = new_session(&mut source);
    let start = session
        .request_start(capabilities(adapter), catalog, full_settings(), true)
        .expect("start");
    let AvActionExecution::Acknowledged(delayed_ack) = start
        .execute_source(&mut session, &mut source)
        .expect("native start")
    else {
        panic!("start must acknowledge");
    };
    handle
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Sleep);
    session
        .poll_source(&mut source)
        .expect("sleep poll")
        .expect("sleep event");
    assert_eq!(session.state(), AvSessionState::TeardownRequired);
    let stop = session.request_stop().expect("stop").expect("stop action");
    let AvActionExecution::Acknowledged(stop_ack) = stop
        .execute_source(&mut session, &mut source)
        .expect("stop dispatch")
    else {
        panic!("stop must acknowledge");
    };
    assert_eq!(handle.lock().expect("fake state").stamps.len(), 3);
    session.complete(stop_ack).expect("stop complete");
    assert!(matches!(
        session.complete(delayed_ack),
        Err(AvCaptureError::StaleOperation)
    ));
}
