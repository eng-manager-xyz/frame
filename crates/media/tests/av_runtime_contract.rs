use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use frame_media::*;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;

fn adapter_id(seed: u8) -> AvAdapterInstanceId {
    AvAdapterInstanceId::from_opaque([seed; 16]).expect("adapter ID")
}

fn device_id(seed: u8) -> AvDeviceId {
    AvDeviceId::from_opaque([seed; 16]).expect("device ID")
}

fn session_id(seed: u8) -> AvSessionId {
    AvSessionId::from_csprng([seed; 16]).expect("session ID")
}

fn audio_format() -> AvFormat {
    AvFormat::Audio(AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::Float32,
    })
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

fn catalog(adapter: AvAdapterInstanceId) -> AvDeviceCatalog {
    AvDeviceCatalog::new(
        adapter,
        1,
        vec![
            AvDeviceDescriptor::new(
                device_id(7),
                AvDeviceGeneration::new(1).expect("generation"),
                AvSourceClass::SystemAudio,
                true,
                PermissionState::Granted,
                NativeRouteClass::Virtual,
                NativeTimestampKind::HostMonotonic,
                vec![audio_format()],
            )
            .expect("system audio descriptor"),
        ],
    )
    .expect("catalog")
}

fn dual_audio_catalog(adapter: AvAdapterInstanceId) -> AvDeviceCatalog {
    AvDeviceCatalog::new(
        adapter,
        1,
        vec![
            AvDeviceDescriptor::new(
                device_id(6),
                AvDeviceGeneration::new(1).expect("generation"),
                AvSourceClass::Microphone,
                true,
                PermissionState::Granted,
                NativeRouteClass::BuiltIn,
                NativeTimestampKind::HostMonotonic,
                vec![audio_format()],
            )
            .expect("microphone descriptor"),
            AvDeviceDescriptor::new(
                device_id(7),
                AvDeviceGeneration::new(1).expect("generation"),
                AvSourceClass::SystemAudio,
                true,
                PermissionState::Granted,
                NativeRouteClass::Virtual,
                NativeTimestampKind::HostMonotonic,
                vec![audio_format()],
            )
            .expect("system audio descriptor"),
        ],
    )
    .expect("dual audio catalog")
}

fn settings() -> AvCaptureSettingsV2 {
    AvCaptureSettingsV2 {
        version: AV_SETTINGS_VERSION,
        microphone: DeviceSelectionV2::Disabled,
        system_audio: DeviceSelectionV2::Pinned {
            id: device_id(7),
            format: audio_format(),
        },
        camera: DeviceSelectionV2::Disabled,
    }
}

fn dual_audio_settings() -> AvCaptureSettingsV2 {
    AvCaptureSettingsV2 {
        version: AV_SETTINGS_VERSION,
        microphone: DeviceSelectionV2::Pinned {
            id: device_id(6),
            format: audio_format(),
        },
        system_audio: DeviceSelectionV2::Pinned {
            id: device_id(7),
            format: audio_format(),
        },
        camera: DeviceSelectionV2::Disabled,
    }
}

fn calibration_samples() -> Vec<CalibrationSample> {
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

#[derive(Default)]
struct FakeState {
    binding: Option<AvOwnerBinding>,
    events: VecDeque<NativeAvEvent>,
    terminal: Option<AvTerminalId>,
    reconciled_terminals: Vec<AvTerminalId>,
    native_release_count: usize,
    reconciliations: usize,
    native_authority_live: bool,
    fail_stop_after_release_once: bool,
    panic_terminal_execute_once: bool,
    calibration_stamp_override: Option<AvSourceStamp>,
}

struct FakeBridge {
    adapter: AvAdapterInstanceId,
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
        assert_eq!(
            self.state.lock().expect("fake state").binding,
            Some(ticket.binding())
        );
        Ok(capabilities(self.adapter))
    }

    fn enumerate(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<AvDeviceCatalog, NativeAvFailure> {
        assert_eq!(
            self.state.lock().expect("fake state").binding,
            Some(ticket.binding())
        );
        Ok(self.catalog.clone())
    }

    fn startup_calibration(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
        stamp: AvSourceStamp,
    ) -> Result<NativeAvCalibrationBatch, NativeAvFailure> {
        let state = self.state.lock().expect("fake state");
        assert_eq!(state.binding, Some(ticket.binding()));
        let returned_stamp = state.calibration_stamp_override.unwrap_or(stamp);
        drop(state);
        NativeAvCalibrationBatch::new(returned_stamp, calibration_samples()).map_err(|_| {
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
        state.reconciliations += 1;
        state.reconciled_terminals.push(ticket.terminal_id());
        Ok(if state.terminal == Some(ticket.terminal_id()) {
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
        let terminal = ticket.terminal_id();
        if matches!(request, AvNativeRequest::Stop | AvNativeRequest::Cancel) {
            let mut state = self.state.lock().expect("fake state");
            if state.panic_terminal_execute_once {
                state.panic_terminal_execute_once = false;
                drop(state);
                panic!("hostile native terminal panic");
            }
            state.terminal = terminal;
            state.native_release_count += 1;
            state.native_authority_live = false;
            if matches!(request, AvNativeRequest::Stop) && state.fail_stop_after_release_once {
                state.fail_stop_after_release_once = false;
                return Err(NativeAvFailure {
                    code: NativeAvFailureCode::Timeout,
                    retryable: true,
                });
            }
        } else if matches!(request, AvNativeRequest::Start(_)) {
            self.state.lock().expect("fake state").native_authority_live = true;
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

fn started_session(
    seed: u8,
) -> (
    BoundNativeAvBridge<FakeBridge>,
    AvCaptureSession,
    Arc<Mutex<FakeState>>,
    AvDeviceCatalog,
) {
    let adapter = adapter_id(seed);
    let live_catalog = catalog(adapter);
    started_session_with(seed, live_catalog, settings())
}

fn started_session_with(
    seed: u8,
    live_catalog: AvDeviceCatalog,
    capture_settings: AvCaptureSettingsV2,
) -> (
    BoundNativeAvBridge<FakeBridge>,
    AvCaptureSession,
    Arc<Mutex<FakeState>>,
    AvDeviceCatalog,
) {
    let adapter = live_catalog.adapter();
    let (bridge, state) = FakeBridge::new(adapter, live_catalog.clone());
    let mut source =
        BoundNativeAvBridge::new(bridge, session_id(seed)).expect("bound native source");
    let mut session = AvCaptureSession::new(source.claim_session().expect("session owner"));
    let action = session
        .request_start(
            capabilities(adapter),
            live_catalog.clone(),
            capture_settings,
            false,
        )
        .expect("start request");
    let AvActionExecution::Acknowledged(acknowledgement) = action
        .execute_source(&mut session, &mut source)
        .expect("start execution")
    else {
        panic!("expected start acknowledgement");
    };
    session.complete(acknowledgement).expect("complete start");
    (source, session, state, live_catalog)
}

struct CountingLease {
    retained_bytes: u64,
    payload: Option<AvPayloadBody>,
    released: Arc<AtomicUsize>,
}

impl AvBufferLease for CountingLease {
    fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    fn take_payload(&mut self) -> Option<AvPayloadBody> {
        self.payload.take()
    }

    fn release(self: Box<Self>) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

fn buffer(
    stamp: AvSourceStamp,
    sequence: u64,
    payload: AvPayloadBody,
    retained_bytes: u64,
    released: &Arc<AtomicUsize>,
) -> NativeAvBuffer {
    buffer_with_discontinuity(stamp, sequence, payload, retained_bytes, false, released)
}

fn buffer_with_discontinuity(
    stamp: AvSourceStamp,
    sequence: u64,
    payload: AvPayloadBody,
    retained_bytes: u64,
    discontinuity: bool,
    released: &Arc<AtomicUsize>,
) -> NativeAvBuffer {
    NativeAvBuffer::new(
        stamp,
        NativeAvBufferTiming {
            sequence,
            source_pts_ns: 60_000_000 + sequence * 10_000_000,
            duration_ns: 10_000_000,
            arrival: MonotonicTimeNs::new(165_000_000 + sequence * 10_000_000),
            latency: SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
            discontinuity,
        },
        audio_format(),
        Box::new(CountingLease {
            retained_bytes,
            payload: Some(payload),
            released: Arc::clone(released),
        }),
    )
    .expect("native buffer")
}

fn corrected_input(
    source: &mut BoundNativeAvBridge<FakeBridge>,
    session: &mut AvCaptureSession,
    state: &Arc<Mutex<FakeState>>,
    buffer: NativeAvBuffer,
    now_ns: u64,
) -> AvAppSrcInput {
    state
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(buffer));
    session
        .poll_source(source)
        .expect("native poll")
        .expect("buffer outcome");
    session
        .pop_buffer(AvSourceClass::SystemAudio, MonotonicTimeNs::new(now_ns))
        .expect("queue pop")
        .expect("corrected buffer")
        .into_appsrc_input()
        .expect("appsrc input")
}

fn graph_spec(catalog: &AvDeviceCatalog) -> AvPipelineGraphSpec {
    AvPipelineGraphSpec::negotiate(catalog, settings(), false).expect("A/V graph spec")
}

const EOS_STRESS_ITERATIONS: u16 = 500;

fn stress_seed(iteration: u16) -> u8 {
    60_u8.saturating_add(u8::try_from(iteration % 190).expect("bounded stress seed"))
}

fn wait_for_release_count(released: &AtomicUsize, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while released.load(Ordering::SeqCst) != expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(released.load(Ordering::SeqCst), expected);
}

fn assert_pipeline_null(pipeline: &gst::Pipeline) {
    let (transition, current, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    assert!(transition.is_ok());
    assert_eq!(current, gst::State::Null);
}

#[test]
fn startup_calibration_is_bounded_and_one_shot_per_stream_epoch() {
    let (mut source, mut session, _, _) = started_session(31);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");

    assert!(matches!(
        NativeAvCalibrationBatch::new(stamp, calibration_samples()[..2].to_vec()),
        Err(AvCaptureError::InvalidCalibrationCount)
    ));
    let mut too_many = calibration_samples();
    while too_many.len() <= MAX_CALIBRATION_SAMPLES {
        let index = u64::try_from(too_many.len()).expect("sample count");
        too_many.push(CalibrationSample {
            master_arrival: MonotonicTimeNs::new(105_000_000 + index * 10_000_000),
            source_pts_ns: index * 10_000_000,
            latency: SourceLatency {
                reported_ns: 5_000_000,
                confidence: LatencyConfidence::Measured,
            },
        });
    }
    assert!(matches!(
        NativeAvCalibrationBatch::new(stamp, too_many),
        Err(AvCaptureError::InvalidCalibrationCount)
    ));

    let batch = source
        .startup_calibration(stamp)
        .expect("native calibration batch");
    session
        .calibrate_source(stamp, AvSyncPolicy::default(), batch.samples())
        .expect("first epoch calibration");
    assert!(matches!(
        session.calibrate_source(stamp, AvSyncPolicy::default(), batch.samples()),
        Err(AvCaptureError::CalibrationAlreadyInstalled)
    ));
}

#[test]
fn bound_bridge_rejects_a_calibration_batch_from_another_stream() {
    let (mut source_a, session_a, state_a, _) = started_session(38);
    let (_, session_b, _, _) = started_session(39);
    let stamp_a = session_a
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source A stamp");
    let stamp_b = session_b
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source B stamp");
    state_a
        .lock()
        .expect("fake state")
        .calibration_stamp_override = Some(stamp_b);

    assert!(matches!(
        source_a.startup_calibration(stamp_a),
        Err(AvCaptureError::StaleSourceStamp)
    ));
}

#[test]
fn audio_and_camera_ingress_budgets_split_exactly_across_three_live_stages() {
    let audio = AvQueueSpec {
        max_buffers: 128,
        max_bytes: 8 * 1024 * 1024,
        max_age_ns: 2_000_000_000,
        backpressure: AvBackpressurePolicy::DropOldest,
        producer_blocks: false,
    };
    let camera = AvQueueSpec {
        max_buffers: 8,
        max_bytes: 128 * 1024 * 1024,
        max_age_ns: 500_000_000,
        backpressure: AvBackpressurePolicy::DropOldest,
        producer_blocks: false,
    };

    for total in [audio, camera] {
        let partition = total.partition_ingress().expect("valid partition");
        for stage in [partition.session, partition.appsrc, partition.downstream] {
            assert!(stage.max_buffers > 0);
            assert!(stage.max_bytes > 0);
            assert!(stage.max_age_ns > 0);
            assert!(!stage.producer_blocks);
        }
        assert_eq!(
            partition.session.max_buffers
                + partition.appsrc.max_buffers
                + partition.downstream.max_buffers,
            total.max_buffers
        );
        assert_eq!(
            partition.session.max_bytes
                + partition.appsrc.max_bytes
                + partition.downstream.max_bytes,
            total.max_bytes
        );
        assert_eq!(
            partition.session.max_age_ns
                + partition.appsrc.max_age_ns
                + partition.downstream.max_age_ns,
            total.max_age_ns
        );
    }

    assert!(matches!(
        AvQueueSpec {
            max_buffers: 2,
            ..audio
        }
        .partition_ingress(),
        Err(AvCaptureError::InvalidQueueSpec)
    ));
}

#[test]
fn production_graph_enforces_both_appsrc_and_queue_bounds_and_confirms_null() {
    let (_, session, _, live_catalog) = started_session(32);
    let spec = graph_spec(&live_catalog);
    let source_spec = spec.sources.first().expect("system audio source");
    let session_queue = session
        .source_ingress_queue_spec(AvSourceClass::SystemAudio)
        .expect("session ingress queue");
    let partition = source_spec
        .queue
        .partition_ingress()
        .expect("three-stage ingress partition");
    assert_eq!(session_queue, partition.session);
    let mut graph = NativeAvGstreamerGraph::build(&spec).expect("native graph");
    let appsrc = graph
        .source_appsrc(AvSourceClass::SystemAudio)
        .expect("typed appsrc");
    let queue = graph
        .source_queue(AvSourceClass::SystemAudio)
        .expect("bounded downstream queue");

    let queue_buffers = queue.property::<u32>("max-size-buffers");
    let queue_bytes = queue.property::<u32>("max-size-bytes");
    let queue_age_ns = queue.property::<u64>("max-size-time");
    assert_eq!(
        u64::from(session_queue.max_buffers) + appsrc.max_buffers() + u64::from(queue_buffers),
        u64::from(source_spec.queue.max_buffers)
    );
    assert_eq!(
        session_queue.max_bytes + appsrc.max_bytes() + u64::from(queue_bytes),
        source_spec.queue.max_bytes
    );
    assert_eq!(
        session_queue.max_age_ns + appsrc.max_time().nseconds() + queue_age_ns,
        source_spec.queue.max_age_ns
    );
    assert_eq!(appsrc.leaky_type(), gst_app::AppLeakyType::Downstream);
    assert!(!appsrc.property::<bool>("block"));
    assert_eq!(
        appsrc.max_buffers(),
        u64::from(partition.appsrc.max_buffers)
    );
    assert_eq!(appsrc.max_bytes(), partition.appsrc.max_bytes);
    assert_eq!(appsrc.max_time().nseconds(), partition.appsrc.max_age_ns);
    assert_eq!(queue_buffers, u32::from(partition.downstream.max_buffers));
    assert_eq!(u64::from(queue_bytes), partition.downstream.max_bytes);
    assert_eq!(queue_age_ns, partition.downstream.max_age_ns);

    graph.start_playing().expect("Playing state");
    assert_eq!(graph.state(), NativeAvGraphState::Playing);
    assert!(matches!(
        graph.poll_bus(0),
        Err(NativeAvGraphFailure::InvalidPollLimit)
    ));
    assert!(matches!(
        graph.start_playing(),
        Err(NativeAvGraphFailure::StateChange)
    ));
    let mut first_buffer = gst::Buffer::from_slice(vec![0_u8; 8 * 480]);
    let first_buffer_ref = first_buffer
        .get_mut()
        .expect("new graph-test buffer is uniquely owned");
    first_buffer_ref.set_pts(gst::ClockTime::ZERO);
    first_buffer_ref.set_duration(gst::ClockTime::from_mseconds(10));
    appsrc
        .push_buffer(first_buffer)
        .expect("first graph buffer");
    graph.request_eos().expect("appsrc EOS");
    assert_eq!(graph.state(), NativeAvGraphState::EosRequested);

    let deadline = Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        let report = graph.poll_bus(32).expect("bounded bus poll");
        if let Some(terminal) = report.terminal {
            break Some(terminal);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(terminal, Some(NativeAvGraphTerminal::EndOfStream));
    graph.confirm_null().expect("confirmed Null");
    assert_eq!(graph.state(), NativeAvGraphState::Null);
}

#[derive(Debug, PartialEq, Eq)]
struct OpaqueHandle(u64);

#[test]
fn cpu_appsrc_preserves_lease_lifetime_and_distinguishes_transfer_failures() {
    prepare_runtime().expect("GStreamer runtime");
    let (mut source, mut session, state, _) = started_session(33);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");
    let batch = source
        .startup_calibration(stamp)
        .expect("calibration batch");
    session
        .calibrate_source(stamp, AvSyncPolicy::default(), batch.samples())
        .expect("calibration");

    let element = gst::parse::launch(concat!(
        "appsrc name=source is-live=true do-timestamp=false block=false format=time ",
        "max-buffers=4 max-bytes=4096 max-time=1000000000 leaky-type=downstream ",
        "caps=\"audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2\" ",
        "! queue name=ingress max-size-buffers=4 max-size-bytes=4096 ",
        "max-size-time=1000000000 leaky=downstream ",
        "! appsink name=sink sync=false async=false enable-last-sample=false max-buffers=4 drop=false"
    ))
    .expect("test pipeline");
    let pipeline = element.downcast::<gst::Pipeline>().expect("pipeline");
    let appsrc = pipeline
        .by_name("source")
        .expect("source")
        .downcast::<gst_app::AppSrc>()
        .expect("typed source");
    let queue = pipeline.by_name("ingress").expect("bounded queue");
    let appsink = pipeline
        .by_name("sink")
        .expect("sink")
        .downcast::<gst_app::AppSink>()
        .expect("typed sink");
    pipeline
        .set_state(gst::State::Playing)
        .expect("start test pipeline");
    let (transition, current, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    assert!(transition.is_ok());
    assert_eq!(current, gst::State::Playing);
    let stage_budget = AvQueueSpec {
        max_buffers: 4,
        max_bytes: 4096,
        max_age_ns: 1_000_000_000,
        backpressure: AvBackpressurePolicy::DropOldest,
        producer_blocks: false,
    };
    let mut adapter = NativeAvAppSrc::new(
        appsrc.clone(),
        queue.clone(),
        stamp,
        audio_format(),
        stage_budget,
        stage_budget,
    )
    .expect("bounded CPU appsrc");
    let opaque_released = Arc::new(AtomicUsize::new(0));
    let bytes_released = Arc::new(AtomicUsize::new(0));
    let overload_released = Arc::new(AtomicUsize::new(0));
    let downstream_released = Arc::new(AtomicUsize::new(0));

    let opaque = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer(
            stamp,
            1,
            AvPayloadBody::Opaque(Box::new(OpaqueHandle(9))),
            16,
            &opaque_released,
        ),
        175_000_000,
    );
    let rejected = adapter.push(opaque).expect_err("opaque rejection");
    assert_eq!(rejected.rejection(), Some(AvAppSrcRejection::OpaquePayload));
    assert_eq!(opaque_released.load(Ordering::SeqCst), 0);
    let rejected = rejected.into_rejected_input().expect("returned input");
    assert_eq!(
        rejected.payload().opaque::<OpaqueHandle>(),
        Some(&OpaqueHandle(9))
    );
    rejected.release();
    assert_eq!(opaque_released.load(Ordering::SeqCst), 1);

    let bytes = vec![0_u8; 8 * 480];
    let input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            2,
            AvPayloadBody::Bytes(bytes.clone()),
            u64::try_from(bytes.len()).expect("payload length"),
            true,
            &bytes_released,
        ),
        185_000_000,
    );
    adapter.push(input).expect("byte push");
    let sample = appsink
        .try_pull_sample(gst::ClockTime::from_seconds(5))
        .expect("downstream sample");
    assert_eq!(bytes_released.load(Ordering::SeqCst), 0);
    let downstream = sample.buffer().expect("sample buffer");
    assert_eq!(
        downstream.pts().map(gst::ClockTime::nseconds),
        Some(180_000_000)
    );
    assert_eq!(
        downstream.duration().map(gst::ClockTime::nseconds),
        Some(10_000_000)
    );
    assert!(downstream.flags().contains(gst::BufferFlags::DISCONT));
    assert_eq!(
        downstream
            .map_readable()
            .expect("readable sample")
            .as_slice(),
        bytes
    );
    drop(sample);

    // The queue's overrun signal is the exact loss observation used in
    // production. Multiple observations coalesce, while the next transferred
    // buffer carries a discontinuity marker.
    queue.emit_by_name::<()>("overrun", &[]);
    queue.emit_by_name::<()>("overrun", &[]);
    assert!(adapter.take_overload_observation());
    assert!(!adapter.take_overload_observation());
    let overload_bytes = vec![1_u8; 8 * 480];
    let input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            3,
            AvPayloadBody::Bytes(overload_bytes.clone()),
            u64::try_from(overload_bytes.len()).expect("payload length"),
            false,
            &overload_released,
        ),
        195_000_000,
    );
    adapter.push(input).expect("post-overload byte push");
    let sample = appsink
        .try_pull_sample(gst::ClockTime::from_seconds(5))
        .expect("post-overload downstream sample");
    assert!(
        sample
            .buffer()
            .expect("post-overload sample buffer")
            .flags()
            .contains(gst::BufferFlags::DISCONT)
    );
    drop(sample);

    appsrc.end_of_stream().expect("EOS request");
    let final_bytes = vec![0_u8; 8 * 480];
    let input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            4,
            AvPayloadBody::Bytes(final_bytes.clone()),
            u64::try_from(final_bytes.len()).expect("payload length"),
            true,
            &downstream_released,
        ),
        205_000_000,
    );
    let consumed = adapter.push(input).expect_err("post-EOS push failure");
    assert_eq!(
        consumed.downstream_code(),
        Some(AvAppSrcDownstreamFailure::EndOfStream)
    );
    assert!(consumed.into_rejected_input().is_none());

    pipeline
        .set_state(gst::State::Null)
        .expect("test pipeline Null");
    let (_, current, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    assert_eq!(current, gst::State::Null);
    wait_for_release_count(&opaque_released, 1);
    wait_for_release_count(&bytes_released, 1);
    wait_for_release_count(&overload_released, 1);
    wait_for_release_count(&downstream_released, 1);
}

#[test]
fn appsrc_pressure_is_observed_without_a_newer_than_minimum_runtime_drop_counter() {
    prepare_runtime().expect("GStreamer runtime");
    let (mut source, mut session, state, _) = started_session(45);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");
    let batch = source
        .startup_calibration(stamp)
        .expect("calibration batch");
    session
        .calibrate_source(stamp, AvSyncPolicy::default(), batch.samples())
        .expect("calibration");

    let element = gst::parse::launch(concat!(
        "appsrc name=source is-live=true do-timestamp=false block=false format=time ",
        "max-buffers=1 max-bytes=4096 max-time=1000000000 leaky-type=downstream ",
        "caps=\"audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2\" ",
        "! identity sleep-time=500000 ",
        "! queue name=ingress max-size-buffers=4 max-size-bytes=16384 ",
        "max-size-time=1000000000 leaky=downstream ",
        "! appsink name=sink sync=false async=false enable-last-sample=false max-buffers=4 drop=false"
    ))
    .expect("pressure pipeline");
    let pipeline = element.downcast::<gst::Pipeline>().expect("pipeline");
    let appsrc = pipeline
        .by_name("source")
        .expect("source")
        .downcast::<gst_app::AppSrc>()
        .expect("typed source");
    let queue = pipeline.by_name("ingress").expect("bounded queue");
    let appsink = pipeline
        .by_name("sink")
        .expect("sink")
        .downcast::<gst_app::AppSink>()
        .expect("typed sink");
    let mut adapter = NativeAvAppSrc::new(
        appsrc.clone(),
        queue,
        stamp,
        audio_format(),
        AvQueueSpec {
            max_buffers: 1,
            max_bytes: 4096,
            max_age_ns: 1_000_000_000,
            backpressure: AvBackpressurePolicy::DropOldest,
            producer_blocks: false,
        },
        AvQueueSpec {
            max_buffers: 4,
            max_bytes: 16_384,
            max_age_ns: 1_000_000_000,
            backpressure: AvBackpressurePolicy::DropOldest,
            producer_blocks: false,
        },
    )
    .expect("bounded CPU appsrc");
    pipeline
        .set_state(gst::State::Playing)
        .expect("start pressure pipeline");
    let (transition, current, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    assert!(transition.is_ok());
    assert_eq!(current, gst::State::Playing);

    let released = Arc::new(AtomicUsize::new(0));
    let first_bytes = vec![2_u8; 8 * 480];
    let first_input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            1,
            AvPayloadBody::Bytes(first_bytes.clone()),
            u64::try_from(first_bytes.len()).expect("payload length"),
            false,
            &released,
        ),
        175_000_000,
    );
    adapter.push(first_input).expect("first adapter buffer");
    let drain_deadline = Instant::now() + Duration::from_secs(1);
    while appsrc.current_level_buffers() != 0 && Instant::now() < drain_deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(appsrc.current_level_buffers(), 0);
    let queued_bytes = vec![3_u8; 8 * 480];
    let queued_input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            2,
            AvPayloadBody::Bytes(queued_bytes.clone()),
            u64::try_from(queued_bytes.len()).expect("payload length"),
            false,
            &released,
        ),
        185_000_000,
    );
    adapter.push(queued_input).expect("queued adapter buffer");
    let fill_deadline = Instant::now() + Duration::from_secs(1);
    while appsrc.current_level_buffers() != 1 && Instant::now() < fill_deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(appsrc.current_level_buffers(), 1);

    let bytes = vec![7_u8; 8 * 480];
    let input = corrected_input(
        &mut source,
        &mut session,
        &state,
        buffer_with_discontinuity(
            stamp,
            3,
            AvPayloadBody::Bytes(bytes.clone()),
            u64::try_from(bytes.len()).expect("payload length"),
            false,
            &released,
        ),
        195_000_000,
    );
    adapter.push(input).expect("pressure-observed push");
    assert!(adapter.take_overload_observation());
    assert!(!adapter.take_overload_observation());
    appsrc.end_of_stream().expect("EOS request");

    let first = appsink
        .try_pull_sample(gst::ClockTime::from_seconds(5))
        .expect("first downstream sample");
    let surviving_sample = appsink
        .try_pull_sample(gst::ClockTime::from_seconds(5))
        .expect("post-overload downstream sample");
    assert_eq!(
        first
            .buffer()
            .expect("first buffer")
            .map_readable()
            .expect("readable first buffer")
            .as_slice(),
        first_bytes
    );
    let surviving_buffer = surviving_sample.buffer().expect("surviving buffer");
    assert!(surviving_buffer.flags().contains(gst::BufferFlags::DISCONT));
    assert_eq!(
        surviving_buffer
            .map_readable()
            .expect("readable surviving buffer")
            .as_slice(),
        bytes
    );

    pipeline
        .set_state(gst::State::Null)
        .expect("pressure pipeline Null");
    let (_, current, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    assert_eq!(current, gst::State::Null);
    drop(first);
    drop(surviving_sample);
    wait_for_release_count(&released, 3);
}

#[test]
fn runtime_poll_is_bounded_coalesced_and_reaches_eos_then_null() {
    let (source, session, state, live_catalog) = started_session(34);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let policy = AvRuntimePolicy {
        max_native_events_per_poll: 2,
        max_buffers_per_poll: 1,
        max_bus_messages_per_poll: 4,
        max_diagnostics_per_poll: 2,
        ui_interval_ns: 10_000_000,
        eos_timeout_ns: DEFAULT_AV_RUNTIME_EOS_TIMEOUT_NS,
    };
    let mut runtime =
        NativeAvRuntime::attach(source, session, graph, AvSyncPolicy::default(), policy)
            .expect("attached runtime");
    let released = Arc::new(AtomicUsize::new(0));
    {
        let mut state = state.lock().expect("fake state");
        for sequence in 1..=3 {
            let bytes = vec![0_u8; 8 * 480];
            let retained = u64::try_from(bytes.len()).expect("payload length");
            state.events.push_back(NativeAvEvent::Buffer(buffer(
                stamp,
                sequence,
                AvPayloadBody::Bytes(bytes),
                retained,
                &released,
            )));
        }
    }

    let first = runtime
        .poll(MonotonicTimeNs::new(195_000_000))
        .expect("first poll");
    assert_eq!(first.native_events_polled, 2, "{first:?}");
    assert_eq!(first.buffers_pushed, 1);
    assert!(first.more_work_possible);
    assert!(first.ui_events.len() <= MAX_AV_RUNTIME_UI_EVENTS);
    assert_eq!(
        first
            .ui_events
            .iter()
            .filter(|event| matches!(event, AvUiEvent::Timing { .. }))
            .count(),
        1
    );

    let second = runtime
        .poll(MonotonicTimeNs::new(205_000_000))
        .expect("second poll");
    assert_eq!(second.native_events_polled, 1);
    assert_eq!(second.buffers_pushed, 1);
    let third = runtime
        .poll(MonotonicTimeNs::new(215_000_000))
        .expect("third poll");
    assert_eq!(third.native_events_polled, 0);
    assert_eq!(third.buffers_pushed, 1);

    runtime
        .request_stop(MonotonicTimeNs::new(220_000_000))
        .expect("bounded stop request");
    assert_eq!(runtime.state(), NativeAvRuntimeState::EosRequested);
    let deadline = Instant::now() + Duration::from_secs(5);
    let termination = loop {
        let report = runtime
            .poll(MonotonicTimeNs::new(225_000_000))
            .expect("terminal poll");
        if let Some(termination) = report.termination {
            break Some(termination);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    .expect("bounded EOS completion");
    assert_eq!(termination.outcome, NativeAvRuntimeOutcome::Completed);
    assert_eq!(
        termination.source_teardown,
        NativeAvSourceTeardown::Confirmed
    );
    assert_eq!(
        termination.graph_teardown,
        NativeAvGraphTeardown::NullReached
    );
    assert_eq!(runtime.state(), NativeAvRuntimeState::NullConfirmed);
    assert_eq!(released.load(Ordering::SeqCst), 3);
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn empty_source_serialized_eos_is_clean_under_stress() {
    for iteration in 0..EOS_STRESS_ITERATIONS {
        let seed = stress_seed(iteration);
        let (source, session, state, live_catalog) = started_session(seed);
        let graph =
            NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
        let mut runtime = NativeAvRuntime::attach(
            source,
            session,
            graph,
            AvSyncPolicy::default(),
            AvRuntimePolicy::default(),
        )
        .expect("attached runtime");

        runtime
            .request_stop(MonotonicTimeNs::new(195_000_000))
            .expect("empty-source stop");
        let deadline = Instant::now() + Duration::from_secs(5);
        let termination = loop {
            let report = runtime
                .poll(MonotonicTimeNs::new(200_000_000))
                .expect("terminal poll");
            if let Some(termination) = report.termination {
                break Some(termination);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::yield_now();
        }
        .expect("bounded EOS completion");
        assert_eq!(
            termination.outcome,
            NativeAvRuntimeOutcome::Completed,
            "iteration {iteration}"
        );
        assert_eq!(
            termination.graph_teardown,
            NativeAvGraphTeardown::NullReached,
            "iteration {iteration}"
        );
        assert_eq!(
            state.lock().expect("fake state").native_release_count,
            1,
            "iteration {iteration}"
        );
    }
}

#[test]
fn first_buffer_immediate_stop_uses_the_same_serialized_eos_path() {
    for iteration in 0..EOS_STRESS_ITERATIONS {
        let seed = stress_seed(iteration);
        let (source, session, state, live_catalog) = started_session(seed);
        let stamp = session
            .source_stamp(AvSourceClass::SystemAudio)
            .expect("source stamp");
        let graph =
            NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
        let mut runtime = NativeAvRuntime::attach(
            source,
            session,
            graph,
            AvSyncPolicy::default(),
            AvRuntimePolicy::default(),
        )
        .expect("attached runtime");
        let released = Arc::new(AtomicUsize::new(0));
        let bytes = vec![0_u8; 8 * 480];
        state
            .lock()
            .expect("fake state")
            .events
            .push_back(NativeAvEvent::Buffer(buffer(
                stamp,
                1,
                AvPayloadBody::Bytes(bytes.clone()),
                u64::try_from(bytes.len()).expect("payload length"),
                &released,
            )));

        let pushed = runtime
            .poll(MonotonicTimeNs::new(195_000_000))
            .expect("first-buffer poll");
        assert_eq!(pushed.buffers_pushed, 1, "iteration {iteration}");
        runtime
            .request_stop(MonotonicTimeNs::new(195_000_001))
            .expect("immediate stop");

        let deadline = Instant::now() + Duration::from_secs(5);
        let termination = loop {
            let report = runtime
                .poll(MonotonicTimeNs::new(200_000_000))
                .expect("terminal poll");
            if let Some(termination) = report.termination {
                break Some(termination);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::yield_now();
        }
        .expect("bounded EOS completion");
        assert_eq!(
            termination.outcome,
            NativeAvRuntimeOutcome::Completed,
            "iteration {iteration}"
        );
        assert_eq!(
            termination.graph_teardown,
            NativeAvGraphTeardown::NullReached,
            "iteration {iteration}"
        );
        wait_for_release_count(&released, 1);
        assert_eq!(
            state.lock().expect("fake state").native_release_count,
            1,
            "iteration {iteration}"
        );
    }
}

#[test]
fn one_buffer_poll_budget_rotates_fairly_across_active_sources() {
    let adapter = adapter_id(41);
    let live_catalog = dual_audio_catalog(adapter);
    let (source, session, state, _) =
        started_session_with(41, live_catalog.clone(), dual_audio_settings());
    let microphone_stamp = session
        .source_stamp(AvSourceClass::Microphone)
        .expect("microphone stamp");
    let system_stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("system audio stamp");
    let spec = AvPipelineGraphSpec::negotiate(&live_catalog, dual_audio_settings(), false)
        .expect("dual-source graph spec");
    let graph = NativeAvGstreamerGraph::build(&spec).expect("dual-source native graph");
    let policy = AvRuntimePolicy {
        max_native_events_per_poll: 8,
        max_buffers_per_poll: 1,
        ui_interval_ns: 10_000_000,
        ..AvRuntimePolicy::default()
    };
    let mut runtime =
        NativeAvRuntime::attach(source, session, graph, AvSyncPolicy::default(), policy)
            .expect("attached dual-source runtime");
    let released = Arc::new(AtomicUsize::new(0));
    {
        let mut state = state.lock().expect("fake state");
        for sequence in 1..=3 {
            let bytes = vec![0_u8; 8 * 480];
            state.events.push_back(NativeAvEvent::Buffer(buffer(
                microphone_stamp,
                sequence,
                AvPayloadBody::Bytes(bytes.clone()),
                u64::try_from(bytes.len()).expect("payload length"),
                &released,
            )));
        }
        let bytes = vec![0_u8; 8 * 480];
        state.events.push_back(NativeAvEvent::Buffer(buffer(
            system_stamp,
            1,
            AvPayloadBody::Bytes(bytes.clone()),
            u64::try_from(bytes.len()).expect("payload length"),
            &released,
        )));
    }

    let first = runtime
        .poll(MonotonicTimeNs::new(210_000_000))
        .expect("microphone poll");
    assert!(first.termination.is_none(), "{first:?}");
    assert_eq!(first.buffers_pushed, 1);
    assert!(first.ui_events.iter().any(|event| matches!(
        event,
        AvUiEvent::Timing {
            class: AvSourceClass::Microphone,
            ..
        }
    )));

    // The microphone still has backlog, but the persistent cursor gives the
    // later system-audio source the next one-buffer budget.
    let second = runtime
        .poll(MonotonicTimeNs::new(220_000_000))
        .expect("system-audio poll");
    assert!(second.termination.is_none(), "{second:?}");
    assert_eq!(second.buffers_pushed, 1);
    assert!(second.ui_events.iter().any(|event| matches!(
        event,
        AvUiEvent::Timing {
            class: AvSourceClass::SystemAudio,
            ..
        }
    )));

    let termination = runtime.cancel().expect("bounded cancellation");
    assert_eq!(termination.outcome, NativeAvRuntimeOutcome::Cancelled);
    wait_for_release_count(&released, 4);
}

#[test]
fn downstream_overruns_emit_one_bounded_privacy_safe_runtime_status() {
    const STIMULUS_BUFFERS: u64 = 80;
    let (source, session, state, live_catalog) = started_session(42);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let queue = graph
        .source_queue(AvSourceClass::SystemAudio)
        .expect("downstream queue");
    let policy = AvRuntimePolicy {
        max_native_events_per_poll: 40,
        max_diagnostics_per_poll: 1,
        ui_interval_ns: 10_000_000,
        ..AvRuntimePolicy::default()
    };
    let mut runtime =
        NativeAvRuntime::attach(source, session, graph, AvSyncPolicy::default(), policy)
            .expect("attached runtime");
    let queue_pad = queue.static_pad("src").expect("queue source pad");
    let block_probe = queue_pad
        .add_probe(
            gst::PadProbeType::BLOCK | gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
            |_, _| gst::PadProbeReturn::Ok,
        )
        .expect("blocking queue probe");
    let exact_overruns = Arc::new(AtomicUsize::new(0));
    let overrun_counter = Arc::clone(&exact_overruns);
    let overrun_handler = queue.connect("overrun", false, move |_| {
        overrun_counter.fetch_add(1, Ordering::SeqCst);
        None
    });
    let released = Arc::new(AtomicUsize::new(0));
    {
        let mut state = state.lock().expect("fake state");
        // Exceed the complete appsrc + downstream partition across multiple
        // runtime polls. A single downstream-capacity-plus-one burst is racy:
        // one buffer can already be held by the blocking pad probe and no
        // longer count against the queue's internal level.
        for sequence in 1..=STIMULUS_BUFFERS {
            let bytes = vec![0_u8; 8 * 480];
            state.events.push_back(NativeAvEvent::Buffer(buffer(
                stamp,
                sequence,
                AvPayloadBody::Bytes(bytes.clone()),
                u64::try_from(bytes.len()).expect("payload length"),
                &released,
            )));
        }
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    // Each native batch stays below the 43-buffer session partition and ends
    // at its exact final callback timestamp. This isolates downstream queue
    // pressure without first dropping at the session stage.
    const BATCH_TIMES_NS: [u64; 2] = [565_000_000, 965_000_000];
    const BATCH_COUNTS: [u16; 2] = [40, 40];
    // These exact timestamps avoid advancing the session's observed monotonic
    // time beyond the next batch's first callback.
    let mut batch_index = 0_usize;
    let mut now_ns = BATCH_TIMES_NS[batch_index];
    let report = loop {
        let report = runtime
            .poll(MonotonicTimeNs::new(now_ns))
            .expect("bounded overload poll");
        assert!(report.termination.is_none(), "{report:?}");
        let surfaced = report.diagnostics.iter().any(|diagnostic| {
            diagnostic.version == AV_DIAGNOSTIC_VERSION
                && diagnostic.class == Some(AvSourceClass::SystemAudio)
                && diagnostic.code == AvStableCode::IngressOverload
        });
        if exact_overruns.load(Ordering::SeqCst) > 0 && surfaced {
            break report;
        }
        assert!(Instant::now() < deadline, "queue overrun was not surfaced");
        if batch_index < BATCH_TIMES_NS.len() {
            assert_eq!(
                report.native_events_polled, BATCH_COUNTS[batch_index],
                "native batch {batch_index}"
            );
            batch_index += 1;
        }
        if batch_index < BATCH_TIMES_NS.len() {
            now_ns = BATCH_TIMES_NS[batch_index];
        } else {
            now_ns = now_ns.saturating_add(10_000_000);
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(report.diagnostics.len(), 1);
    assert!(!report.diagnostics_truncated);
    assert_eq!(
        report
            .ui_events
            .iter()
            .filter(|event| matches!(
                event,
                AvUiEvent::SourceStatus {
                    class: AvSourceClass::SystemAudio,
                    code: AvStableCode::IngressOverload,
                }
            ))
            .count(),
        1
    );

    queue_pad.remove_probe(block_probe);
    queue.disconnect(overrun_handler);

    let termination = runtime.cancel().expect("bounded cancellation");
    assert_eq!(termination.outcome, NativeAvRuntimeOutcome::Cancelled);
    wait_for_release_count(
        &released,
        usize::try_from(STIMULUS_BUFFERS).expect("stimulus count fits usize"),
    );
}

#[test]
fn lost_eos_expires_on_the_runtime_monotonic_deadline_and_confirms_null() {
    assert!(matches!(
        AvRuntimePolicy {
            eos_timeout_ns: 0,
            ..AvRuntimePolicy::default()
        }
        .validate(),
        Err(NativeAvRuntimeError::InvalidPolicy)
    ));
    assert!(matches!(
        AvRuntimePolicy {
            eos_timeout_ns: MAX_AV_RUNTIME_EOS_TIMEOUT_NS + 1,
            ..AvRuntimePolicy::default()
        }
        .validate(),
        Err(NativeAvRuntimeError::InvalidPolicy)
    ));

    let (source, session, state, live_catalog) = started_session(40);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let timeout_ns = 10_000_000;
    let policy = AvRuntimePolicy {
        eos_timeout_ns: timeout_ns,
        ..AvRuntimePolicy::default()
    };
    let mut runtime =
        NativeAvRuntime::attach(source, session, graph, AvSyncPolicy::default(), policy)
            .expect("attached runtime");
    let appsrc = runtime
        .graph()
        .source_appsrc(AvSourceClass::SystemAudio)
        .expect("system appsrc");
    assert_eq!(appsrc.current_level_buffers(), 0);
    assert!(
        appsrc
            .static_pad("src")
            .expect("appsrc source pad")
            .sticky_event::<gst::event::Segment>(0)
            .is_none()
    );
    let stop_time = 200_000_000;
    runtime
        .request_stop(MonotonicTimeNs::new(stop_time))
        .expect("bounded stop request");
    assert_eq!(appsrc.current_level_buffers(), 0);

    // Consume the actual EOS message outside the runtime to model a lost bus
    // notification. The expiration assertion below advances only the injected
    // monotonic clock; it does not depend on a harness sleep timeout.
    let bus = runtime.graph().pipeline().bus().expect("pipeline bus");
    let lost_eos = bus
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(5),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("terminal bus message");
    assert!(matches!(lost_eos.view(), gst::MessageView::Eos(_)));
    assert_eq!(
        appsrc
            .static_pad("src")
            .expect("appsrc source pad")
            .sticky_event::<gst::event::Segment>(0)
            .expect("serialized empty-source segment")
            .segment()
            .format(),
        gst::Format::Time
    );

    let before_deadline = runtime
        .poll(MonotonicTimeNs::new(stop_time + timeout_ns - 1))
        .expect("pre-deadline poll");
    assert!(before_deadline.termination.is_none());
    let expired = runtime
        .poll(MonotonicTimeNs::new(stop_time + timeout_ns))
        .expect("deadline poll");
    assert_eq!(
        expired.termination,
        Some(NativeAvTermination {
            outcome: NativeAvRuntimeOutcome::Failed(NativeAvRuntimeFailure::EosDeadlineExceeded,),
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        })
    );
    assert_eq!(runtime.state(), NativeAvRuntimeState::Failed);
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_failure_quiesces_the_started_source_and_confirms_graph_null() {
    let (source, session, state, live_catalog) = started_session(35);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let invalid_policy = AvRuntimePolicy {
        max_native_events_per_poll: 0,
        ..AvRuntimePolicy::default()
    };

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        invalid_policy,
    )
    .expect_err("invalid attachment must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_rejects_a_live_gstreamer_bound_mutated_after_graph_construction() {
    let (source, session, state, live_catalog) = started_session(44);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let appsrc = graph
        .source_appsrc(AvSourceClass::SystemAudio)
        .expect("typed appsrc");
    appsrc.set_max_buffers(appsrc.max_buffers() + 1);

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect_err("mutated live bound must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_rejects_a_queue_mutated_to_suppress_overrun_signals() {
    let (source, session, state, live_catalog) = started_session(46);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let queue = graph
        .source_queue(AvSourceClass::SystemAudio)
        .expect("downstream queue");
    queue.set_property("silent", true);

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect_err("silent queue must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_rejects_mutated_live_mixer_liveness_properties() {
    let (source, session, state, live_catalog) = started_session(47);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let mixer = graph
        .pipeline()
        .by_name("audio_mixer")
        .expect("audio mixer");
    assert!(!mixer.property::<bool>("force-live"));
    assert!(!mixer.property::<bool>("ignore-inactive-pads"));
    mixer.set_property("ignore-inactive-pads", true);

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect_err("mutated mixer liveness must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_rejects_a_mutated_live_appsink_eos_contract() {
    let (source, session, state, live_catalog) = started_session(49);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let sink = graph
        .mixed_audio_sink()
        .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
        .expect("mixed audio appsink");
    assert!(!sink.is_wait_on_eos());
    sink.set_wait_on_eos(true);

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect_err("mutated appsink EOS contract must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn attach_rejects_live_appsrc_caps_mutation() {
    let (source, session, state, live_catalog) = started_session(48);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let appsrc = graph
        .source_appsrc(AvSourceClass::SystemAudio)
        .expect("typed appsrc");
    let wrong_caps = gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("layout", "interleaved")
        .field("rate", 44_100_i32)
        .field("channels", 2_i32)
        .build();
    appsrc.set_caps(Some(&wrong_caps));

    let error = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect_err("mutated live caps must fail closed");
    assert!(matches!(
        error,
        NativeAvRuntimeError::Attach {
            failure: NativeAvRuntimeFailure::CaptureContract,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn poll_contract_error_is_terminal_and_releases_the_queued_lease_once() {
    let (source, session, state, live_catalog) = started_session(36);
    let stamp = session
        .source_stamp(AvSourceClass::SystemAudio)
        .expect("source stamp");
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let mut runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    let released = Arc::new(AtomicUsize::new(0));
    let bytes = vec![0_u8; 8 * 480];
    let retained = u64::try_from(bytes.len()).expect("payload length");
    state
        .lock()
        .expect("fake state")
        .events
        .push_back(NativeAvEvent::Buffer(buffer(
            stamp,
            1,
            AvPayloadBody::Bytes(bytes),
            retained,
            &released,
        )));

    // The poll clock predates the accepted native arrival, which is a hard
    // contract violation rather than retryable empty work.
    let report = runtime
        .poll(MonotonicTimeNs::new(100_000_000))
        .expect("terminal report");
    assert_eq!(
        report.termination,
        Some(NativeAvTermination {
            outcome: NativeAvRuntimeOutcome::Failed(NativeAvRuntimeFailure::CaptureContract),
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        })
    );
    assert_eq!(runtime.state(), NativeAvRuntimeState::Failed);
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert_eq!(state.lock().expect("fake state").native_release_count, 1);
}

#[test]
fn lost_stop_ack_reconciles_the_stable_stop_without_double_release() {
    let (source, session, state, live_catalog) = started_session(37);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let mut runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    state
        .lock()
        .expect("fake state")
        .fail_stop_after_release_once = true;

    let error = runtime
        .request_stop(MonotonicTimeNs::new(200_000_000))
        .expect_err("lost stop acknowledgement");
    assert!(matches!(
        error,
        NativeAvRuntimeError::SourceControl {
            failure: NativeAvRuntimeFailure::Native(NativeAvFailureCode::Timeout),
            source_teardown: NativeAvSourceTeardown::Confirmed,
            teardown: NativeAvGraphTeardown::NullReached,
        }
    ));
    assert_eq!(runtime.state(), NativeAvRuntimeState::Failed);
    let state = state.lock().expect("fake state");
    assert_eq!(state.native_release_count, 1);
    assert_eq!(state.reconciliations, 2);
}

#[test]
fn dropping_a_running_runtime_quiesces_native_authority_and_confirms_null() {
    let (source, session, state, live_catalog) = started_session(50);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let pipeline = graph.pipeline().clone();
    let runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    assert_eq!(runtime.state(), NativeAvRuntimeState::Playing);
    assert!(state.lock().expect("fake state").native_authority_live);

    drop(runtime);

    let state = state.lock().expect("fake state");
    assert!(!state.native_authority_live);
    assert_eq!(state.native_release_count, 1);
    assert_eq!(state.reconciliations, 1);
    assert!(state.terminal.is_some());
    drop(state);
    assert_pipeline_null(&pipeline);
}

#[test]
fn dropping_an_eos_requested_runtime_does_not_double_release_native_authority() {
    let (source, session, state, live_catalog) = started_session(51);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let pipeline = graph.pipeline().clone();
    let mut runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    runtime
        .request_stop(MonotonicTimeNs::new(200_000_000))
        .expect("bounded stop request");
    assert_eq!(runtime.state(), NativeAvRuntimeState::EosRequested);

    drop(runtime);

    let state = state.lock().expect("fake state");
    assert!(!state.native_authority_live);
    assert_eq!(state.native_release_count, 1);
    assert_eq!(state.reconciliations, 1);
    assert!(state.terminal.is_some());
    drop(state);
    assert_pipeline_null(&pipeline);
}

#[test]
fn hostile_drop_never_unwinds_and_leaves_unconfirmed_native_authority_sticky() {
    let (source, session, state, live_catalog) = started_session(52);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let pipeline = graph.pipeline().clone();
    let runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    state
        .lock()
        .expect("fake state")
        .panic_terminal_execute_once = true;

    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(runtime)));
    assert!(dropped.is_ok(), "runtime Drop must contain adapter panics");

    let state = state.lock().expect("fake state");
    assert!(state.native_authority_live);
    assert_eq!(state.native_release_count, 0);
    assert_eq!(state.reconciliations, 1);
    assert!(state.terminal.is_none());
    drop(state);
    assert_pipeline_null(&pipeline);
}

#[test]
fn explicit_quiesce_retries_the_same_sticky_terminal_after_an_unconfirmed_attempt() {
    let (source, session, state, live_catalog) = started_session(53);
    let graph = NativeAvGstreamerGraph::build(&graph_spec(&live_catalog)).expect("native graph");
    let mut runtime = NativeAvRuntime::attach(
        source,
        session,
        graph,
        AvSyncPolicy::default(),
        AvRuntimePolicy::default(),
    )
    .expect("attached runtime");
    state
        .lock()
        .expect("fake state")
        .panic_terminal_execute_once = true;

    let first = runtime.quiesce().expect("contained hostile quiesce");
    assert_eq!(
        first,
        NativeAvTermination {
            outcome: NativeAvRuntimeOutcome::Failed(NativeAvRuntimeFailure::CaptureContract),
            source_teardown: NativeAvSourceTeardown::ContractFailed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    );
    assert_eq!(runtime.state(), NativeAvRuntimeState::Failed);
    assert_eq!(runtime.session().state(), AvSessionState::Stopping);

    let second = runtime.quiesce().expect("same terminal retry");
    assert_eq!(
        second,
        NativeAvTermination {
            outcome: NativeAvRuntimeOutcome::Cancelled,
            source_teardown: NativeAvSourceTeardown::Confirmed,
            graph_teardown: NativeAvGraphTeardown::NullReached,
        }
    );
    assert_eq!(runtime.state(), NativeAvRuntimeState::NullConfirmed);
    let state = state.lock().expect("fake state");
    assert!(!state.native_authority_live);
    assert_eq!(state.native_release_count, 1);
    assert_eq!(state.reconciliations, 2);
    assert_eq!(state.reconciled_terminals.len(), 2);
    assert_eq!(state.reconciled_terminals[0], state.reconciled_terminals[1]);
}
