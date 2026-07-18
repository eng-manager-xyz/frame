use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    time::Duration,
};

use frame_media::*;

type TestIngress = ScreenCaptureIngress<Vec<u8>, Vec<u8>>;

fn source_instance(marker: u8) -> ScreenSourceInstanceId {
    ScreenSourceInstanceId::new([marker; 16]).expect("source instance")
}

fn session_id(marker: u8) -> ScreenSessionId {
    ScreenSessionId::from_csprng([marker; 16]).expect("session id")
}

fn target_id(kind: ScreenTargetKind, marker: u8) -> ScreenTargetId {
    ScreenTargetId::new(kind, [marker; 16]).expect("target id")
}

fn target_binding(
    source: ScreenSourceInstanceId,
    generation: u64,
    epoch: u64,
    kind: ScreenTargetKind,
    marker: u8,
) -> ScreenTargetBinding {
    ScreenTargetBinding::new(
        source,
        generation,
        ScreenTargetEpoch::new(epoch).expect("target epoch"),
        target_id(kind, marker),
    )
    .expect("target binding")
}

fn transform() -> DisplayGeometryTransform {
    DisplayGeometryTransform::new(
        LogicalRect::new(-100, 50, 320, 180).expect("logical bounds"),
        PhysicalRect::new(1_000, -500, 640, 360).expect("physical bounds"),
        DpiScale::new(2, 1).expect("DPI scale"),
        Rotation::Degrees0,
    )
    .expect("transform")
}

fn display_target(
    source: ScreenSourceInstanceId,
    generation: u64,
    epoch: u64,
) -> ScreenTargetDescriptor {
    ScreenTargetDescriptor::display(
        target_binding(source, generation, epoch, ScreenTargetKind::Display, 1),
        transform(),
    )
    .expect("display target")
}

fn window_target(
    source: ScreenSourceInstanceId,
    generation: u64,
    epoch: u64,
    marker: u8,
) -> ScreenTargetDescriptor {
    ScreenTargetDescriptor::window(
        target_binding(source, generation, epoch, ScreenTargetKind::Window, marker),
        LogicalRect::new(-20, 60, 100, 80).expect("window bounds"),
    )
    .expect("window target")
}

const fn output_spec() -> VideoFrameSpec {
    VideoFrameSpec {
        width: 320,
        height: 180,
        pixel_format: PixelFormat::Bgra8,
        color_space: ColorSpace::Srgb,
        nominal_frame_duration_ns: 33_333_333,
        memory: FrameMemory::Cpu,
    }
}

fn current_source() -> PlatformScreenSource {
    match ScreenCapturePlatform::current() {
        ScreenCapturePlatform::MacOs => PlatformScreenSource::ScreenCaptureKit,
        ScreenCapturePlatform::Windows => PlatformScreenSource::WindowsGraphicsCapture,
        ScreenCapturePlatform::Linux => PlatformScreenSource::PipeWirePortal,
        ScreenCapturePlatform::Unsupported => panic!("desktop platform required"),
    }
}

fn capability_spec(
    source: ScreenSourceInstanceId,
    generation: u64,
    control_sequence: u64,
) -> ScreenSourceCapabilitySpec {
    ScreenSourceCapabilitySpec {
        contract_version: SCREEN_CAPTURE_CONTRACT_VERSION,
        source: current_source(),
        source_instance: source,
        topology_generation: generation,
        control_epoch: ScreenControlEpoch::new(7).expect("control epoch"),
        control_sequence,
        targets: ScreenTargetKinds::none()
            .with(ScreenTargetKind::Display)
            .with(ScreenTargetKind::Window)
            .with(ScreenTargetKind::Region),
        cursor_modes: ScreenCursorModes::none()
            .with(CursorCaptureMode::Hidden)
            .with(CursorCaptureMode::EmbeddedInFrame)
            .with(CursorCaptureMode::Metadata),
        cursor_image_metadata: true,
        cursor_click_metadata: true,
        cursor_desktop_logical_coordinates: true,
        cursor_frame_physical_coordinates: true,
        frame_profiles: vec![ScreenFrameProfile {
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            memory: FrameMemory::Cpu,
            max_width: 3_840,
            max_height: 2_160,
            max_frames_per_second: 120,
        }],
        permission_preflight: true,
        topology_events: true,
        target_recovery: true,
        protected_content_events: true,
        window_exclusion: true,
        max_excluded_windows: 8,
        bounded_appsrc_ingress: true,
    }
}

fn capabilities(
    source: ScreenSourceInstanceId,
    generation: u64,
    control_sequence: u64,
) -> ScreenSourceCapabilities {
    ScreenSourceCapabilities::new(capability_spec(source, generation, control_sequence))
        .expect("capabilities")
}

fn queue_policy(
    max_frames: u16,
    max_bytes: u64,
    max_age_ns: u64,
    overflow: CaptureQueueOverflow,
) -> ScreenCaptureQueuePolicy {
    ScreenCaptureQueuePolicy::new(max_frames, max_bytes, max_age_ns, overflow)
        .expect("queue policy")
}

fn default_queue() -> ScreenCaptureQueuePolicy {
    queue_policy(8, 800, 1_000, CaptureQueueOverflow::DropOldest)
}

fn request(
    target: ScreenTargetDescriptor,
    excluded_windows: Vec<ScreenTargetBinding>,
    cursor: CursorPolicy,
    queue: ScreenCaptureQueuePolicy,
    recovery: TargetRecoveryPolicy,
    protected_content: ProtectedContentPolicy,
) -> ScreenCaptureRequest {
    ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
        target,
        output: output_spec(),
        cursor,
        excluded_windows,
        queue,
        recovery,
        protected_content,
    })
    .expect("request")
}

fn cursor_policy(
    mode: CursorCaptureMode,
    include_image_revision: bool,
    include_clicks: bool,
) -> CursorPolicy {
    CursorPolicy::new(mode, include_image_revision, include_clicks).expect("cursor policy")
}

fn initial_contract(
    cursor: CursorPolicy,
    queue: ScreenCaptureQueuePolicy,
    recovery: TargetRecoveryPolicy,
    protected_content: ProtectedContentPolicy,
) -> (
    ScreenSourceCapabilities,
    ScreenTargetSnapshot,
    NegotiatedScreenCapture,
) {
    let source = source_instance(1);
    let selected = display_target(source, 1, 1);
    let excluded = window_target(source, 1, 1, 2);
    let catalog = ScreenTargetSnapshot::new(source, 1, vec![selected.clone(), excluded.clone()])
        .expect("catalog");
    let capabilities = capabilities(source, 1, 1);
    let negotiated = negotiate_screen_capture(
        &capabilities,
        &catalog,
        request(
            selected,
            vec![excluded.binding()],
            cursor,
            queue,
            recovery,
            protected_content,
        ),
    )
    .expect("negotiation");
    (capabilities, catalog, negotiated)
}

fn control_stamp(source: ScreenSourceInstanceId, sequence: u64) -> ScreenControlStamp {
    ScreenControlStamp::new(
        source,
        ScreenControlEpoch::new(7).expect("control epoch"),
        sequence,
    )
    .expect("control stamp")
}

fn observation(
    source: ScreenSourceInstanceId,
    sequence: u64,
    permission: PermissionPreflight,
) -> ScreenPermissionObservation {
    ScreenPermissionObservation {
        stamp: control_stamp(source, sequence),
        permission,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TicketRecord {
    kind: ScreenOperationKind,
    operation_id: ScreenOperationId,
    session_binding: ScreenSourceSessionBinding,
    stream: ScreenStreamStamp,
    predecessor_stream: Option<ScreenStreamStamp>,
    catalog_generation: u64,
}

type SharedNativeStreams = Rc<RefCell<BTreeMap<ScreenSourceSessionBinding, ScreenStreamStamp>>>;

struct DummySource {
    capabilities: ScreenSourceCapabilities,
    catalog: ScreenTargetSnapshot,
    pending_topology: RefCell<Option<(ScreenSourceCapabilities, ScreenTargetSnapshot)>>,
    pending_events: RefCell<VecDeque<ScreenSourceEvent<Vec<u8>, Vec<u8>>>>,
    preflight_observation: RefCell<Option<ScreenPermissionObservation>>,
    session_binding: Option<ScreenSourceSessionBinding>,
    native_streams: RefCell<SharedNativeStreams>,
    records: Vec<TicketRecord>,
    active_stream: Option<ScreenStreamStamp>,
    delayed_failure: Option<ScreenSourceFailure>,
    fail_next: Cell<bool>,
    next_failure: Cell<Option<(ScreenSourceFailureCode, bool)>>,
    enumeration_failure: Cell<Option<ScreenSourceFailureCode>>,
    preflight_failure: Cell<Option<ScreenSourceFailureCode>>,
    request_failure: Cell<Option<ScreenSourceFailureCode>>,
    poll_failure: Cell<Option<ScreenSourceFailureCode>>,
    enumeration_calls: usize,
    bind_calls: usize,
    preflight_calls: usize,
    poll_calls: usize,
    stop_calls: usize,
    stop_budget_cancelled: Vec<bool>,
    stopped_native_streams: Vec<Option<ScreenStreamStamp>>,
}

impl DummySource {
    fn new(capabilities: ScreenSourceCapabilities, catalog: ScreenTargetSnapshot) -> Self {
        Self::with_native_streams(
            capabilities,
            catalog,
            Rc::new(RefCell::new(BTreeMap::new())),
        )
    }

    fn with_native_streams(
        capabilities: ScreenSourceCapabilities,
        catalog: ScreenTargetSnapshot,
        native_streams: SharedNativeStreams,
    ) -> Self {
        Self {
            capabilities,
            catalog,
            pending_topology: RefCell::new(None),
            pending_events: RefCell::new(VecDeque::new()),
            preflight_observation: RefCell::new(None),
            session_binding: None,
            native_streams: RefCell::new(native_streams),
            records: Vec::new(),
            active_stream: None,
            delayed_failure: None,
            fail_next: Cell::new(false),
            next_failure: Cell::new(None),
            enumeration_failure: Cell::new(None),
            preflight_failure: Cell::new(None),
            request_failure: Cell::new(None),
            poll_failure: Cell::new(None),
            enumeration_calls: 0,
            bind_calls: 0,
            preflight_calls: 0,
            poll_calls: 0,
            stop_calls: 0,
            stop_budget_cancelled: Vec::new(),
            stopped_native_streams: Vec::new(),
        }
    }

    fn update_topology(
        &self,
        capabilities: ScreenSourceCapabilities,
        catalog: ScreenTargetSnapshot,
    ) {
        self.pending_topology.replace(Some((capabilities, catalog)));
    }

    fn update_catalog(&self, catalog: ScreenTargetSnapshot) {
        let capabilities = self.pending_topology.borrow().as_ref().map_or_else(
            || self.capabilities.clone(),
            |(capabilities, _)| capabilities.clone(),
        );
        self.update_topology(capabilities, catalog);
    }

    fn use_native_streams(&self, native_streams: SharedNativeStreams) {
        self.native_streams.replace(native_streams);
    }

    fn native_stream(&self, binding: ScreenSourceSessionBinding) -> Option<ScreenStreamStamp> {
        self.native_streams.borrow().borrow().get(&binding).copied()
    }

    fn fail_next_operation(&self) {
        self.fail_next.set(true);
    }

    fn set_next_failure(&self, code: ScreenSourceFailureCode, retryable: bool) {
        self.next_failure.set(Some((code, retryable)));
    }

    fn set_enumeration_failure(&self, code: ScreenSourceFailureCode) {
        self.enumeration_failure.set(Some(code));
    }

    fn set_preflight_failure(&self, code: ScreenSourceFailureCode) {
        self.preflight_failure.set(Some(code));
    }

    fn set_request_failure(&self, code: ScreenSourceFailureCode) {
        self.request_failure.set(Some(code));
    }

    fn set_poll_failure(&self, code: ScreenSourceFailureCode) {
        self.poll_failure.set(Some(code));
    }

    fn set_preflight_observation(&self, observation: ScreenPermissionObservation) {
        self.preflight_observation.replace(Some(observation));
    }

    fn queue_event(&self, event: ScreenSourceEvent<Vec<u8>, Vec<u8>>) {
        self.pending_events.borrow_mut().push_back(event);
    }

    fn receive_ticket(
        &mut self,
        expected_kind: ScreenOperationKind,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        budget.check()?;
        assert_eq!(ticket.kind(), expected_kind);
        assert_eq!(ticket.negotiated().source(), current_source());
        assert_eq!(
            ticket.catalog_generation(),
            ticket.negotiated().catalog().generation()
        );
        let record = TicketRecord {
            kind: ticket.kind(),
            operation_id: ticket.operation_id(),
            session_binding: ticket.session_binding(),
            stream: ticket.stream(),
            predecessor_stream: ticket.predecessor_stream(),
            catalog_generation: ticket.catalog_generation(),
        };
        assert_eq!(self.session_binding, Some(record.session_binding));
        let injected_failure = self.next_failure.take().or_else(|| {
            self.fail_next
                .get()
                .then_some((ScreenSourceFailureCode::NativeOperationFailed, true))
        });
        self.fail_next.set(false);
        let (failure_code, failure_retryable) =
            injected_failure.unwrap_or((ScreenSourceFailureCode::NativeOperationFailed, true));
        let failure = ticket.failure(failure_code, failure_retryable);
        self.records.push(record);
        self.delayed_failure = Some(failure.clone());
        if injected_failure.is_some() {
            return Err(failure);
        }
        match expected_kind {
            ScreenOperationKind::Start | ScreenOperationKind::Reconfigure => {
                self.active_stream = Some(record.stream);
                self.native_streams
                    .borrow()
                    .borrow_mut()
                    .insert(record.session_binding, record.stream);
            }
            ScreenOperationKind::Stop => {
                let native_stream = self
                    .native_streams
                    .borrow()
                    .borrow()
                    .get(&record.session_binding)
                    .copied();
                assert!(native_stream.is_none_or(|stream| {
                    stream == record.stream || Some(stream) == record.predecessor_stream
                }));
                self.native_streams
                    .borrow()
                    .borrow_mut()
                    .remove(&record.session_binding);
                self.active_stream = None;
            }
        }
        Ok(())
    }

    fn record(&self, index: usize) -> TicketRecord {
        self.records[index]
    }

    fn frame_for(
        &self,
        stream: ScreenStreamStamp,
        sequence: u64,
        pts_ns: u64,
        retained_bytes: u64,
        cursor: Option<CursorFrameMetadata>,
    ) -> ScreenFrame<Vec<u8>> {
        ScreenFrame::new(
            ScreenFrameEnvelope {
                stream,
                sequence,
                timestamp: FrameTimestamp::new(pts_ns, 1).expect("timestamp"),
                spec: output_spec(),
                retained_bytes,
                cursor,
            },
            vec![0; usize::try_from(retained_bytes.min(16)).expect("payload length")],
        )
        .expect("frame")
    }

    fn frame(
        &self,
        sequence: u64,
        pts_ns: u64,
        retained_bytes: u64,
        cursor: Option<CursorFrameMetadata>,
    ) -> ScreenFrame<Vec<u8>> {
        self.frame_for(
            self.active_stream.expect("active source stream"),
            sequence,
            pts_ns,
            retained_bytes,
            cursor,
        )
    }

    fn cursor_image_for(
        &self,
        stream: ScreenStreamStamp,
        revision: u64,
    ) -> ScreenCursorImage<Vec<u8>> {
        ScreenCursorImage::new(stream, cursor_descriptor(revision), vec![0; 64])
    }

    fn cursor_image(&self, revision: u64) -> ScreenCursorImage<Vec<u8>> {
        self.cursor_image_for(self.active_stream.expect("active source stream"), revision)
    }
}

impl ScreenCaptureSource for DummySource {
    type FramePayload = Vec<u8>;
    type CursorImagePayload = Vec<u8>;

    fn source_instance(&self) -> ScreenSourceInstanceId {
        self.capabilities.source_instance()
    }

    fn session_binding(&self) -> Option<ScreenSourceSessionBinding> {
        self.session_binding
    }

    fn bind_session(
        &mut self,
        ticket: ScreenSourceSessionTicket,
    ) -> Result<(), ScreenCaptureError> {
        self.bind_calls = self.bind_calls.saturating_add(1);
        let binding = ticket.binding();
        if binding.source_instance() != self.capabilities.source_instance() {
            return Err(ScreenCaptureError::SourceSessionOwnershipMismatch);
        }
        match self.session_binding {
            Some(current) if current != binding => {
                Err(ScreenCaptureError::SourceSessionOwnershipMismatch)
            }
            Some(_) => Ok(()),
            None => {
                self.session_binding = Some(binding);
                Ok(())
            }
        }
    }

    fn capabilities<'a>(
        &'a self,
        ticket: &ScreenSourceCallTicket<'_>,
    ) -> &'a ScreenSourceCapabilities {
        assert_eq!(self.session_binding, Some(ticket.binding()));
        &self.capabilities
    }

    fn preflight(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        assert_eq!(self.session_binding, Some(ticket.binding()));
        self.preflight_calls = self.preflight_calls.saturating_add(1);
        budget.check()?;
        if let Some(code) = self.preflight_failure.take() {
            return Err(ScreenSourceFailure::new(code, true));
        }
        Ok(self
            .preflight_observation
            .get_mut()
            .take()
            .unwrap_or_else(|| {
                observation(
                    self.capabilities.source_instance(),
                    2,
                    PermissionPreflight::Granted,
                )
            }))
    }

    fn request_permission(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        assert_eq!(self.session_binding, Some(ticket.binding()));
        budget.check()?;
        if let Some(code) = self.request_failure.take() {
            return Err(ScreenSourceFailure::new(code, true));
        }
        Ok(observation(
            self.capabilities.source_instance(),
            3,
            PermissionPreflight::Granted,
        ))
    }

    fn enumerate_targets(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenTargetSnapshot, ScreenSourceFailure> {
        assert_eq!(self.session_binding, Some(ticket.binding()));
        budget.check()?;
        self.enumeration_calls = self.enumeration_calls.saturating_add(1);
        if let Some((capabilities, catalog)) = self.pending_topology.get_mut().take() {
            self.capabilities = capabilities;
            self.catalog = catalog;
        }
        if let Some(code) = self.enumeration_failure.get() {
            return Err(ScreenSourceFailure::new(code, true));
        }
        Ok(self.catalog.clone())
    }

    fn start(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.receive_ticket(ScreenOperationKind::Start, ticket, budget)
    }

    fn reconfigure(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.receive_ticket(ScreenOperationKind::Reconfigure, ticket, budget)
    }

    fn poll_event(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> ScreenSourcePollResult<Self::FramePayload, Self::CursorImagePayload> {
        assert_eq!(self.session_binding, Some(ticket.binding()));
        self.poll_calls = self.poll_calls.saturating_add(1);
        budget.check()?;
        if let Some(code) = self.poll_failure.take() {
            return Err(ScreenSourceFailure::new(code, true));
        }
        Ok(self.pending_events.get_mut().pop_front())
    }

    fn stop(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.stop_calls = self.stop_calls.saturating_add(1);
        self.stop_budget_cancelled
            .push(budget.cancellation().is_cancelled());
        self.stopped_native_streams.push(self.active_stream);
        self.receive_ticket(ScreenOperationKind::Stop, ticket, budget)
    }
}

fn execute(
    session: &ScreenCaptureSession,
    action: &mut ScreenSessionAction,
    source: &mut BoundScreenCaptureSource<DummySource>,
) -> Result<Option<ScreenOperationAck>, ScreenOperationExecutionError> {
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    action.execute_source(session, source, &budget)
}

trait TestSourceEventIngress {
    fn handle_source_event(
        &mut self,
        session: &mut ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<DummySource>,
        event: ScreenSourceEvent<Vec<u8>, Vec<u8>>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenIngressOutcome, ScreenCaptureError>;
}

impl TestSourceEventIngress for TestIngress {
    fn handle_source_event(
        &mut self,
        session: &mut ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<DummySource>,
        event: ScreenSourceEvent<Vec<u8>, Vec<u8>>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenIngressOutcome, ScreenCaptureError> {
        source.queue_event(event);
        let poll_cancellation = CancellationToken::new();
        let budget = ScreenOperationBudget::new(&poll_cancellation, Duration::from_secs(1))
            .expect("source-event poll budget");
        let envelope = source
            .poll_owned_event(&budget)
            .expect("queued source event must not fail")
            .expect("queued source event must be returned");
        self.apply_source_event(session, envelope, now_ns, cancellation)
    }
}

struct Harness {
    session: ScreenCaptureSession,
    ingress: TestIngress,
    source: BoundScreenCaptureSource<DummySource>,
}

impl Harness {
    fn apply_source_transition(
        &mut self,
        event: ScreenSourceEvent<Vec<u8>, Vec<u8>>,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        let outcome = self.ingress.handle_source_event(
            &mut self.session,
            &mut self.source,
            event,
            0,
            &CancellationToken::new(),
        )?;
        let ScreenIngressOutcome::Session(transition) = outcome else {
            return Err(ScreenCaptureError::InvalidSessionTransition);
        };
        Ok(*transition)
    }
}

fn ready_harness_with(
    cursor: CursorPolicy,
    queue: ScreenCaptureQueuePolicy,
    recovery: TargetRecoveryPolicy,
    protected_content: ProtectedContentPolicy,
    session_marker: u8,
) -> Harness {
    let (capabilities, catalog, negotiated) =
        initial_contract(cursor, queue, recovery, protected_content);
    let mut source = BoundScreenCaptureSource::new(
        DummySource::new(capabilities, catalog),
        session_id(session_marker),
    )
    .expect("bound source");
    let mut session =
        ScreenCaptureSession::new(negotiated, source.binding()).expect("bound session");
    let mut ingress = TestIngress::new(&session).expect("ingress");
    let preflight = session.initial_action();
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let transition = ingress
        .execute_control_action(&mut session, &preflight, &mut source, &budget)
        .expect("granted preflight");
    assert_eq!(transition.transition.to, ScreenCapturePhase::Ready);
    assert_eq!(source.preflight_calls, 1);
    Harness {
        session,
        ingress,
        source,
    }
}

fn ready_harness(session_marker: u8) -> Harness {
    ready_harness_with(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
        session_marker,
    )
}

fn start_and_ack(harness: &mut Harness) -> ScreenOperationAck {
    let mut transition = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start requested");
    assert_eq!(transition.transition.to, ScreenCapturePhase::Starting);
    assert!(matches!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::Start { .. }
    ));
    let ack = execute(
        &harness.session,
        &mut transition.transition.action,
        &mut harness.source,
    )
    .expect("source start")
    .expect("start acknowledgement");
    harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("start acknowledged");
    ack
}

fn capturing_harness(session_marker: u8) -> Harness {
    let mut harness = ready_harness(session_marker);
    start_and_ack(&mut harness);
    assert_eq!(harness.session.phase(), ScreenCapturePhase::Capturing);
    harness
}

fn shared_backend_capturing_harnesses() -> (Harness, Harness) {
    let mut session_a = ready_harness(1);
    let mut session_b = ready_harness(2);
    let native_streams = Rc::new(RefCell::new(BTreeMap::new()));
    session_a
        .source
        .use_native_streams(Rc::clone(&native_streams));
    session_b.source.use_native_streams(native_streams);
    start_and_ack(&mut session_a);
    start_and_ack(&mut session_b);
    (session_a, session_b)
}

fn topology(
    generation: u64,
    target_epoch: u64,
    control_sequence: u64,
) -> (
    ScreenSourceCapabilities,
    ScreenTargetSnapshot,
    ScreenTargetDescriptor,
) {
    let source = source_instance(1);
    let selected = display_target(source, generation, target_epoch);
    let excluded = window_target(source, generation, 1, 2);
    let catalog = ScreenTargetSnapshot::new(source, generation, vec![selected.clone(), excluded])
        .expect("topology catalog");
    (
        capabilities(source, generation, control_sequence),
        catalog,
        selected,
    )
}

fn reconfigure_event(
    harness: &mut Harness,
    generation: u64,
    target_epoch: u64,
    sequence: u64,
) -> ScreenTargetChange {
    let (capabilities, catalog, selected) = topology(generation, target_epoch, 2);
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    ScreenTargetChange::reconfigured(
        ScreenTopologyStamp::new(source_instance(1), generation, sequence).expect("topology stamp"),
        selected,
        capabilities,
        catalog,
    )
    .expect("reconfiguration event")
}

fn unrelated_hotplug_event(
    harness: &mut Harness,
    selected_epoch: u64,
    excluded_epoch: u64,
) -> ScreenTargetChange {
    let source = source_instance(1);
    let selected = display_target(source, 2, selected_epoch);
    let excluded = window_target(source, 2, excluded_epoch, 2);
    let unrelated = window_target(source, 2, 1, 4);
    let catalog = ScreenTargetSnapshot::new(source, 2, vec![selected, excluded, unrelated.clone()])
        .expect("unrelated hotplug catalog");
    let capabilities = capabilities(source, 2, 2);
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    ScreenTargetChange::added(
        ScreenTopologyStamp::new(source, 2, 1).expect("hotplug stamp"),
        unrelated,
        capabilities,
        catalog,
    )
    .expect("unrelated hotplug event")
}

fn cursor_descriptor(revision: u64) -> CursorImageDescriptor {
    CursorImageDescriptor::new(revision, 4, 4, 0, 0, PixelFormat::Bgra8, 64)
        .expect("cursor descriptor")
}

fn cursor_metadata(
    target: &ScreenTargetDescriptor,
    revision: Option<u64>,
    click: bool,
) -> CursorFrameMetadata {
    normalize_screen_cursor(
        target,
        output_spec(),
        cursor_policy(CursorCaptureMode::Metadata, revision.is_some(), click),
        RawCursorObservation {
            visible: true,
            position: RawCursorPosition::TargetFramePhysical { x: 10, y: 20 },
            image_revision: revision,
            primary_click: click,
            secondary_click: false,
        },
    )
    .expect("cursor normalization")
    .expect("metadata")
}

#[test]
fn identities_are_nonzero_collision_resistant_inputs_and_redacted() {
    assert_eq!(
        ScreenSessionId::from_csprng([0; 16]),
        Err(ScreenCaptureError::InvalidSessionId)
    );
    assert_eq!(
        ScreenSourceInstanceId::new([0; 16]),
        Err(ScreenCaptureError::InvalidSourceInstanceId)
    );
    let session = session_id(9);
    let source = source_instance(9);
    assert!(!format!("{session:?}{source:?}").contains("0909"));
}

#[test]
fn bound_source_bootstraps_capabilities_catalog_and_session_under_one_owner() {
    let source = source_instance(1);
    let selected = display_target(source, 1, 1);
    let excluded = window_target(source, 1, 1, 2);
    let catalog = ScreenTargetSnapshot::new(source, 1, vec![selected.clone(), excluded.clone()])
        .expect("catalog");
    let capabilities = capabilities(source, 1, 1);
    let mut bound = BoundScreenCaptureSource::new(
        DummySource::new(capabilities.clone(), catalog.clone()),
        session_id(9),
    )
    .expect("pre-negotiation source binding");
    assert_eq!(bound.bind_calls, 1);
    assert_eq!(bound.enumeration_calls, 0);
    assert_eq!(bound.capabilities(), &capabilities);
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let live_catalog = bound
        .enumerate_targets(&budget)
        .expect("bound bootstrap enumeration");
    assert_eq!(live_catalog, catalog);
    assert_eq!(bound.enumeration_calls, 1);
    let negotiated = negotiate_screen_capture(
        bound.capabilities(),
        &live_catalog,
        request(
            selected,
            vec![excluded.binding()],
            cursor_policy(CursorCaptureMode::Metadata, true, true),
            default_queue(),
            TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
            ProtectedContentPolicy::SuspendUntilClear,
        ),
    )
    .expect("bound negotiation");
    let session =
        ScreenCaptureSession::new(negotiated, bound.binding()).expect("same-owner session");
    assert_eq!(session.session_id(), session_id(9));
    assert_eq!(bound.session_binding(), Some(bound.binding()));
    assert!(format!("{:?}{bound:?}", bound.binding()).contains("<redacted>"));
}

#[test]
fn preflight_transition_requires_the_bound_native_control_result() {
    let (capabilities, catalog, negotiated) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source =
        BoundScreenCaptureSource::new(DummySource::new(capabilities, catalog), session_id(1))
            .expect("bound source");
    let mut session =
        ScreenCaptureSession::new(negotiated, source.binding()).expect("bound session");
    let mut ingress = TestIngress::new(&session).expect("ingress");
    let phase_before = session.phase();
    let diagnostics_before = session.diagnostics();
    let action = session.initial_action();
    assert_eq!(phase_before, ScreenCapturePhase::AwaitingPreflight);
    assert_eq!(source.preflight_calls, 0);
    assert_eq!(session.diagnostics(), diagnostics_before);

    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let transition = ingress
        .execute_control_action(&mut session, &action, &mut source, &budget)
        .expect("owner-bound preflight");

    assert_eq!(source.preflight_calls, 1);
    assert_eq!(transition.transition.to, ScreenCapturePhase::Ready);
    assert_eq!(session.phase(), ScreenCapturePhase::Ready);
}

#[test]
fn safe_bound_wrapper_swap_moves_adapter_and_owner_as_one_unit() {
    let (capabilities_a, catalog_a, _) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let (capabilities_b, catalog_b, _) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut bound_a =
        BoundScreenCaptureSource::new(DummySource::new(capabilities_a, catalog_a), session_id(1))
            .expect("bound A");
    let mut bound_b =
        BoundScreenCaptureSource::new(DummySource::new(capabilities_b, catalog_b), session_id(2))
            .expect("bound B");
    let binding_a = bound_a.binding();
    let binding_b = bound_b.binding();

    std::mem::swap(&mut bound_a, &mut bound_b);

    assert_eq!(bound_a.binding(), binding_b);
    assert_eq!(bound_a.session_binding(), Some(binding_b));
    assert_eq!(bound_b.binding(), binding_a);
    assert_eq!(bound_b.session_binding(), Some(binding_a));
    assert_eq!(bound_a.bind_calls, 1);
    assert_eq!(bound_b.bind_calls, 1);
}

#[test]
fn region_snapshot_requires_the_canonical_containing_display_transform() {
    let source = source_instance(1);
    let display = display_target(source, 1, 1);
    let forged_transform = DisplayGeometryTransform::new(
        transform().logical_bounds(),
        PhysicalRect::new(999, -500, 640, 360).expect("forged physical bounds"),
        DpiScale::new(2, 1).expect("scale"),
        Rotation::Degrees0,
    )
    .expect("internally valid forged transform");
    let region = ScreenTargetDescriptor::region(
        target_binding(source, 1, 1, ScreenTargetKind::Region, 3),
        display.binding(),
        LogicalRect::new(-90, 60, 50, 40).expect("region"),
        forged_transform,
    )
    .expect("region descriptor");
    assert_eq!(
        ScreenTargetSnapshot::new(source, 1, vec![display, region]),
        Err(ScreenCaptureError::ForgedRegionTransform)
    );
}

#[test]
fn exact_negotiation_preserves_nonblocking_appsrc_and_native_lease_lifetime() {
    let (_, _, negotiated) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let ingress = negotiated.ingress();
    assert_eq!(ingress.factory, "appsrc");
    assert!(!ingress.block);
    assert!(!ingress.do_timestamp);
    assert_eq!(
        ingress.buffer_lifetime,
        AppSrcBufferLifetime::OwnedUntilDownstreamRelease
    );
    assert_eq!(ingress.frame_spec, output_spec());
}

#[test]
fn source_execution_revalidates_the_complete_live_catalog_and_consumes_the_ticket() {
    let mut harness = ready_harness(1);
    let mut transition = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let unrelated = window_target(source_instance(1), 1, 1, 4);
    let mut targets = harness.source.catalog.targets().to_vec();
    targets.push(unrelated);
    harness.source.update_catalog(
        ScreenTargetSnapshot::new(source_instance(1), 1, targets).expect("changed live catalog"),
    );

    assert_eq!(
        execute(
            &harness.session,
            &mut transition.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::Contract(
            ScreenCaptureError::SourceCatalogChanged
        ))
    );
    assert_eq!(
        execute(
            &harness.session,
            &mut transition.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::TicketConsumed)
    );
    assert!(harness.source.records.is_empty());
    assert_eq!(harness.session.phase(), ScreenCapturePhase::Starting);
}

#[test]
fn stop_bypasses_failed_enumeration_after_permission_revocation() {
    let mut harness = capturing_harness(1);
    harness
        .source
        .set_enumeration_failure(ScreenSourceFailureCode::PermissionDenied);
    let enumeration_calls = harness.source.enumeration_calls;
    let mut revoked = harness
        .apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            3,
            PermissionPreflight::Revoked(SettingsGuidance::OpenSystemSettings),
        )))
        .expect("permission revocation");
    let ack = execute(
        &harness.session,
        &mut revoked.transition.action,
        &mut harness.source,
    )
    .expect("stop bypasses enumeration")
    .expect("stop ack");
    assert_eq!(ack.kind(), ScreenOperationKind::Stop);
    assert_eq!(harness.source.enumeration_calls, enumeration_calls);
    assert_eq!(harness.source.stop_calls, 1);
}

#[test]
fn enumeration_failure_is_bound_and_recovers_through_the_exact_stop_transition() {
    let mut harness = ready_harness(1);
    harness
        .source
        .set_enumeration_failure(ScreenSourceFailureCode::AdapterUnavailable);
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let failure = match execute(
        &harness.session,
        &mut start.transition.action,
        &mut harness.source,
    ) {
        Err(ScreenOperationExecutionError::Source(failure)) => failure,
        other => panic!("expected bound enumeration failure, got {other:?}"),
    };
    assert!(failure.operation_id().is_some());
    assert!(failure.stream().is_some());
    let mut failed = harness
        .ingress
        .apply_operation_failure(&mut harness.session, failure)
        .expect("bound failure transition");
    assert!(matches!(
        failed.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::Source(
            ScreenSourceFailureCode::AdapterUnavailable
        ))
    ));
    assert!(matches!(
        failed.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    execute(
        &harness.session,
        &mut failed.transition.action,
        &mut harness.source,
    )
    .expect("failure stop bypasses enumeration")
    .expect("stop ack");
    assert_eq!(harness.source.stop_calls, 1);
}

#[test]
fn adapter_receives_exact_kind_bound_ticket_and_replay_is_rejected() {
    let mut harness = ready_harness(1);
    let mut transition = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let ack = execute(
        &harness.session,
        &mut transition.transition.action,
        &mut harness.source,
    )
    .expect("source execution")
    .expect("ack");
    let record = harness.source.record(0);
    assert_eq!(record.kind, ScreenOperationKind::Start);
    assert_eq!(record.operation_id, ack.operation_id());
    assert_eq!(record.stream, ack.stream());
    assert_eq!(record.catalog_generation, 1);
    assert_eq!(
        execute(
            &harness.session,
            &mut transition.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::TicketConsumed)
    );

    harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("matching ack");
    assert_eq!(harness.session.phase(), ScreenCapturePhase::Capturing);
    assert_eq!(harness.ingress.active_stream(), Some(record.stream));
    assert_eq!(
        harness
            .ingress
            .complete_operation(&mut harness.session, ack),
        Err(ScreenCaptureError::MismatchedOperationAck)
    );
}

#[test]
fn superseded_unexecuted_start_action_cannot_mint_a_stale_ticket() {
    let mut harness = ready_harness(1);
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let mut stop = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Stop)
        .expect("stop supersedes start");
    assert_eq!(
        execute(
            &harness.session,
            &mut start.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::StaleOperationAction)
    );
    assert!(harness.source.records.is_empty());

    let ack = execute(
        &harness.session,
        &mut stop.transition.action,
        &mut harness.source,
    )
    .expect("stop execution")
    .expect("stop ack");
    let stopped = harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("stop ack");
    assert_eq!(stopped.transition.to, ScreenCapturePhase::Stopped);
}

#[test]
fn newer_grant_while_starting_preserves_start_and_emits_no_duplicate_command() {
    let mut harness = ready_harness(1);
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let granted = harness
        .apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            3,
            PermissionPreflight::Granted,
        )))
        .expect("benign grant");
    assert_eq!(granted.transition.to, ScreenCapturePhase::Starting);
    assert_eq!(
        granted.transition.action.source_command(),
        ScreenSourceCommand::None
    );

    let ack = execute(
        &harness.session,
        &mut start.transition.action,
        &mut harness.source,
    )
    .expect("source start")
    .expect("start ack");
    harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("ack remains required");
    assert_eq!(harness.session.phase(), ScreenCapturePhase::Capturing);
    assert_eq!(harness.source.records.len(), 1);
}

#[test]
fn newer_grant_while_reconfiguring_preserves_reconfigure_and_matching_ack_requirement() {
    let mut harness = capturing_harness(1);
    let change = reconfigure_event(&mut harness, 2, 2, 1);
    let mut reconfigure = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("reconfiguration transition");
    assert_eq!(reconfigure.transition.to, ScreenCapturePhase::Reconfiguring);
    assert!(matches!(
        reconfigure.transition.action.source_command(),
        ScreenSourceCommand::Reconfigure { .. }
    ));

    let granted = harness
        .apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            3,
            PermissionPreflight::Granted,
        )))
        .expect("benign grant");
    assert_eq!(granted.transition.to, ScreenCapturePhase::Reconfiguring);
    assert_eq!(
        granted.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(harness.ingress.active_stream(), None);

    let ack = execute(
        &harness.session,
        &mut reconfigure.transition.action,
        &mut harness.source,
    )
    .expect("reconfigure")
    .expect("ack");
    harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("matching reconfigure ack");
    assert_eq!(harness.session.phase(), ScreenCapturePhase::Capturing);
    assert_eq!(harness.ingress.active_stream(), Some(ack.stream()));
}

#[test]
fn selected_reconfigure_during_pending_stop_never_reopens_capture() {
    let mut harness = capturing_harness(1);
    let mut stop = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Stop)
        .expect("stop requested");
    assert_eq!(stop.transition.to, ScreenCapturePhase::Stopping);
    assert!(matches!(
        stop.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));

    let change = reconfigure_event(&mut harness, 2, 2, 1);
    let changed = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("newer selected reconfiguration");
    assert_eq!(changed.transition.to, ScreenCapturePhase::Stopping);
    assert_eq!(
        changed.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(
        harness.session.pending_operation_kind(),
        Some(ScreenOperationKind::Stop)
    );

    let stop_ack = execute(
        &harness.session,
        &mut stop.transition.action,
        &mut harness.source,
    )
    .expect("stop")
    .expect("stop ack");
    let stopped = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("original stop ack");
    assert_eq!(stopped.transition.to, ScreenCapturePhase::Stopped);
    assert_eq!(
        stopped.transition.action.source_command(),
        ScreenSourceCommand::None
    );
}

#[test]
fn selected_reconfigure_while_starting_issues_stop_and_rejects_held_start_ack() {
    let mut harness = ready_harness(1);
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let held_start_ack = execute(
        &harness.session,
        &mut start.transition.action,
        &mut harness.source,
    )
    .expect("source start")
    .expect("held start ack");

    let change = reconfigure_event(&mut harness, 2, 2, 1);
    let mut changed = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("target changed during start");
    assert_eq!(changed.transition.to, ScreenCapturePhase::Stopping);
    assert!(matches!(
        changed.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert_eq!(
        harness
            .ingress
            .complete_operation(&mut harness.session, held_start_ack),
        Err(ScreenCaptureError::MismatchedOperationAck)
    );

    let stop_ack = execute(
        &harness.session,
        &mut changed.transition.action,
        &mut harness.source,
    )
    .expect("stop superseded start")
    .expect("stop ack");
    let restarted = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("stop ack starts fresh operation");
    assert_eq!(restarted.transition.to, ScreenCapturePhase::Starting);
    assert!(matches!(
        restarted.transition.action.source_command(),
        ScreenSourceCommand::Start { .. }
    ));
    assert_eq!(harness.ingress.active_stream(), None);
}

#[test]
fn frames_are_rejected_before_ack_during_reconfigure_and_after_stream_retirement() {
    let mut harness = ready_harness(1);
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let start_ack = execute(
        &harness.session,
        &mut start.transition.action,
        &mut harness.source,
    )
    .expect("source start")
    .expect("start ack");
    let pre_ack = harness.source.frame(1, 1, 100, None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(pre_ack),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );
    harness
        .ingress
        .complete_operation(&mut harness.session, start_ack)
        .expect("start ack");
    let old_stream = start_ack.stream();

    let change = reconfigure_event(&mut harness, 2, 2, 1);
    let mut reconfigure = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("reconfigure transition");
    let reconfigure_ack = execute(
        &harness.session,
        &mut reconfigure.transition.action,
        &mut harness.source,
    )
    .expect("source reconfigure")
    .expect("reconfigure ack");
    let new_pre_ack = harness.source.frame(1, 2, 100, None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(new_pre_ack),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );
    let old_delayed = harness.source.frame_for(old_stream, 2, 2, 100, None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(old_delayed),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );

    harness
        .ingress
        .complete_operation(&mut harness.session, reconfigure_ack)
        .expect("reconfigure ack");
    let accepted = harness.source.frame(1, 3, 100, None);
    assert!(matches!(
        harness
            .ingress
            .handle_source_event(
                &mut harness.session,
                &mut harness.source,
                ScreenSourceEvent::Frame(accepted),
                3,
                &CancellationToken::new(),
            )
            .expect("current frame"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
    ));
    let old_after_ack = harness.source.frame_for(old_stream, 3, 4, 100, None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(old_after_ack),
            4,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );
}

#[test]
fn equal_local_sequences_from_distinct_session_ids_cannot_cross_ingress() {
    let first = capturing_harness(1);
    let mut second = capturing_harness(2);
    let first_record = first.source.record(0);
    let second_record = second.source.record(0);
    assert_eq!(first_record.operation_id, second_record.operation_id);
    assert_eq!(
        first_record.stream.capture_epoch(),
        second_record.stream.capture_epoch()
    );
    assert_ne!(first_record.stream, second_record.stream);

    let foreign = first.source.frame(1, 1, 100, None);
    assert_eq!(
        second.ingress.handle_source_event(
            &mut second.session,
            &mut second.source,
            ScreenSourceEvent::Frame(foreign),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );
}

#[test]
fn source_failures_require_exact_operation_stream_source_target_and_session_binding() {
    let mut first = capturing_harness(1);
    let mut second = capturing_harness(2);
    let foreign_failure = first
        .source
        .delayed_failure
        .clone()
        .expect("bound start failure");
    assert_eq!(
        second.ingress.handle_source_event(
            &mut second.session,
            &mut second.source,
            ScreenSourceEvent::Failure(foreign_failure),
            0,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::StaleSourceFailure)
    );

    let delayed_old_failure = first
        .source
        .delayed_failure
        .clone()
        .expect("old bound failure");
    let change = reconfigure_event(&mut first, 2, 2, 1);
    let mut reconfigure = first
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("reconfigure");
    let ack = execute(
        &first.session,
        &mut reconfigure.transition.action,
        &mut first.source,
    )
    .expect("execute reconfigure")
    .expect("ack");
    first
        .ingress
        .complete_operation(&mut first.session, ack)
        .expect("activate new stream");
    assert_eq!(
        first.ingress.handle_source_event(
            &mut first.session,
            &mut first.source,
            ScreenSourceEvent::Failure(delayed_old_failure),
            0,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::StaleSourceFailure)
    );
    assert_eq!(first.session.phase(), ScreenCapturePhase::Capturing);
}

#[test]
fn a_ticket_bound_source_failure_fails_only_its_pending_operation() {
    let mut harness = ready_harness(1);
    harness.source.fail_next_operation();
    let mut start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("start transition");
    let failure = match execute(
        &harness.session,
        &mut start.transition.action,
        &mut harness.source,
    ) {
        Err(ScreenOperationExecutionError::Source(failure)) => failure,
        other => panic!("expected bound source failure, got {other:?}"),
    };
    assert!(failure.operation_id().is_some());
    assert!(failure.stream().is_some());
    let failed = harness
        .ingress
        .apply_operation_failure(&mut harness.session, failure)
        .expect("matching failure");
    assert!(matches!(
        failed.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::Source(
            ScreenSourceFailureCode::NativeOperationFailed
        ))
    ));
}

#[test]
fn hidden_cursor_mode_rejects_both_metadata_and_cursor_images() {
    let mut harness = ready_harness_with(
        cursor_policy(CursorCaptureMode::Hidden, false, false),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
        1,
    );
    start_and_ack(&mut harness);
    let metadata = cursor_metadata(harness.session.target(), Some(1), true);
    let frame = harness.source.frame(1, 1, 100, Some(metadata));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorMetadataNotNegotiated)
    );
    let image = harness.source.cursor_image(1);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorImageNotNegotiated)
    );
}

#[test]
fn embedded_cursor_mode_rejects_duplicate_metadata_and_images() {
    let mut harness = ready_harness_with(
        cursor_policy(CursorCaptureMode::EmbeddedInFrame, false, false),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
        1,
    );
    start_and_ack(&mut harness);
    let metadata = cursor_metadata(harness.session.target(), None, false);
    let frame = harness.source.frame(1, 1, 100, Some(metadata));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorMetadataNotNegotiated)
    );
    let image = harness.source.cursor_image(1);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorImageNotNegotiated)
    );
}

#[test]
fn revision_disabled_metadata_rejects_revision_fields_images_and_unrequested_clicks() {
    let mut harness = ready_harness_with(
        cursor_policy(CursorCaptureMode::Metadata, false, false),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
        1,
    );
    start_and_ack(&mut harness);
    let with_revision = cursor_metadata(harness.session.target(), Some(1), false);
    let frame = harness.source.frame(1, 1, 100, Some(with_revision));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorImageNotNegotiated)
    );
    let image = harness.source.cursor_image(1);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorImageNotNegotiated)
    );
    let with_click = cursor_metadata(harness.session.target(), None, true);
    let frame = harness.source.frame(2, 2, 100, Some(with_click));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::CursorClickMetadataNotNegotiated)
    );
}

#[test]
fn revision_required_visible_cursor_needs_the_current_monotonic_image() {
    let mut harness = capturing_harness(1);
    let metadata = cursor_metadata(harness.session.target(), Some(1), true);
    let missing = harness.source.frame(1, 1, 100, Some(metadata));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(missing),
            1,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::MissingCursorImage)
    );

    let image = harness.source.cursor_image(1);
    assert!(matches!(
        harness
            .ingress
            .handle_source_event(
                &mut harness.session,
                &mut harness.source,
                ScreenSourceEvent::CursorImage(image),
                1,
                &CancellationToken::new(),
            )
            .expect("cursor image"),
        ScreenIngressOutcome::CursorImageAccepted
    ));
    let current = harness.source.frame(2, 2, 100, Some(metadata));
    assert!(matches!(
        harness
            .ingress
            .handle_source_event(
                &mut harness.session,
                &mut harness.source,
                ScreenSourceEvent::Frame(current),
                2,
                &CancellationToken::new(),
            )
            .expect("current cursor frame"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
    ));
    let replay_image = harness.source.cursor_image(1);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(replay_image),
            3,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::NonMonotonicCursorImageRevision)
    );
    let future_metadata = cursor_metadata(harness.session.target(), Some(2), true);
    let future = harness.source.frame(3, 3, 100, Some(future_metadata));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(future),
            3,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::MissingCursorImage)
    );
}

#[test]
fn queue_is_bounded_by_frames_and_bytes_without_blocking_the_source() {
    let mut harness = ready_harness_with(
        cursor_policy(CursorCaptureMode::Metadata, false, false),
        queue_policy(2, 200, 1_000, CaptureQueueOverflow::DropOldest),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
        1,
    );
    start_and_ack(&mut harness);
    for (sequence, now) in [(1, 1), (2, 2)] {
        let frame = harness.source.frame(sequence, now, 100, None);
        assert!(matches!(
            harness
                .ingress
                .handle_source_event(
                    &mut harness.session,
                    &mut harness.source,
                    ScreenSourceEvent::Frame(frame),
                    now,
                    &CancellationToken::new(),
                )
                .expect("queued frame"),
            ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
        ));
    }
    let frame = harness.source.frame(3, 3, 100, None);
    assert!(matches!(
        harness
            .ingress
            .handle_source_event(
                &mut harness.session,
                &mut harness.source,
                ScreenSourceEvent::Frame(frame),
                3,
                &CancellationToken::new(),
            )
            .expect("bounded drop-oldest"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::AcceptedAfterDropping {
            frames: 1,
            bytes: 100
        })
    ));
    let diagnostics = harness.ingress.queue_diagnostics();
    assert_eq!(diagnostics.queued_frames, 2);
    assert_eq!(diagnostics.queued_bytes, 200);
    assert_eq!(diagnostics.dropped_oldest, 1);
    let popped = harness
        .ingress
        .try_pop(&mut harness.session, 3, &CancellationToken::new())
        .expect("pop");
    let ScreenIngressPopOutcome::Frame(frame) = popped else {
        panic!("expected frame");
    };
    assert_eq!(frame.sequence(), 2);
}

#[test]
fn cancellation_drains_frame_and_cursor_leases_as_one_ingress_boundary() {
    let mut harness = capturing_harness(1);
    let image = harness.source.cursor_image(1);
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        )
        .expect("cursor image");
    let metadata = cursor_metadata(harness.session.target(), Some(1), false);
    let frame = harness.source.frame(1, 1, 100, Some(metadata));
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        )
        .expect("frame");

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let outcome = harness
        .ingress
        .try_pop(&mut harness.session, 2, &cancellation)
        .expect("cancel");
    let ScreenIngressPopOutcome::Cancelled(mut cancelled) = outcome else {
        panic!("expected cancellation drain");
    };
    assert_eq!(cancelled.transition.to, ScreenCapturePhase::Cancelled);
    let drain = cancelled.drain.expect("single cancellation drain");
    assert_eq!(drain.queue.frames, 1);
    assert_eq!(drain.queue.bytes, 100);
    assert_eq!(drain.cursor.images, 1);
    assert_eq!(drain.cursor.bytes, 64);
    assert_eq!(harness.ingress.cursor_descriptor(), None);
    assert_eq!(harness.ingress.active_stream(), None);
    assert!(matches!(
        cancelled.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    let stop_ack = execute(
        &harness.session,
        &mut cancelled.transition.action,
        &mut harness.source,
    )
    .expect("cancel stop")
    .expect("cancel stop ack");
    let delayed_stop_failure = harness
        .source
        .delayed_failure
        .clone()
        .expect("bound delayed stop failure");
    assert_eq!(harness.source.stop_calls, 1);
    let completed = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(completed.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(completed.drain, None);
    assert_eq!(
        completed.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(harness.session.pending_operation_kind(), None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Failure(delayed_stop_failure),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::InvalidSessionTransition)
    );
    assert_eq!(harness.source.stop_calls, 1);

    let repeated = harness
        .ingress
        .cancel_session(&mut harness.session)
        .expect("idempotent cancel");
    assert_eq!(repeated.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(repeated.drain, None);
    assert_eq!(
        repeated.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(harness.source.stop_calls, 1);
    assert_eq!(harness.ingress.queue_diagnostics().cancellation_drains, 1);
}

#[test]
fn retryable_stop_failure_reissues_stop_without_a_second_cancellation_drain() {
    let mut harness = capturing_harness(1);
    let mut cancelled = harness
        .ingress
        .cancel_session(&mut harness.session)
        .expect("cancel session");
    harness.source.fail_next_operation();
    let failure = match execute(
        &harness.session,
        &mut cancelled.transition.action,
        &mut harness.source,
    ) {
        Err(ScreenOperationExecutionError::Source(failure)) => failure,
        other => panic!("expected retryable stop failure, got {other:?}"),
    };
    assert!(failure.retryable());
    let mut retry = harness
        .ingress
        .apply_operation_failure(&mut harness.session, failure)
        .expect("retry transition");
    assert_eq!(retry.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(retry.drain, None);
    assert!(matches!(
        retry.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    execute(
        &harness.session,
        &mut retry.transition.action,
        &mut harness.source,
    )
    .expect("retry stop")
    .expect("retry stop ack");
    assert_eq!(harness.source.stop_calls, 2);
    assert_eq!(harness.ingress.queue_diagnostics().cancellation_drains, 1);
}

#[test]
fn teardown_ignores_session_token_and_recovers_nonretryable_cancel_deadline_races() {
    let mut harness = capturing_harness(1);
    let mut cancelled = harness
        .ingress
        .cancel_session(&mut harness.session)
        .expect("cancel session");
    let shared_cancellation = CancellationToken::new();
    shared_cancellation.cancel();
    let cancelled_budget = ScreenOperationBudget::new(&shared_cancellation, Duration::from_secs(1))
        .expect("cancelled session budget");

    harness
        .source
        .set_next_failure(ScreenSourceFailureCode::Cancelled, false);
    let cancelled_failure = match cancelled.transition.action.execute_source(
        &harness.session,
        &mut harness.source,
        &cancelled_budget,
    ) {
        Err(ScreenOperationExecutionError::Source(failure)) => failure,
        other => panic!("expected injected cancellation race, got {other:?}"),
    };
    assert!(!cancelled_failure.retryable());
    assert_eq!(harness.source.stop_budget_cancelled, vec![false]);
    let mut retry = harness
        .ingress
        .apply_operation_failure(&mut harness.session, cancelled_failure.clone())
        .expect("terminal cancellation failure retries Stop");
    assert_eq!(retry.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(retry.drain, None);

    harness
        .source
        .set_next_failure(ScreenSourceFailureCode::DeadlineExceeded, false);
    let deadline_failure = match retry.transition.action.execute_source(
        &harness.session,
        &mut harness.source,
        &cancelled_budget,
    ) {
        Err(ScreenOperationExecutionError::Source(failure)) => failure,
        other => panic!("expected injected deadline race, got {other:?}"),
    };
    assert!(!deadline_failure.retryable());
    let mut final_retry = harness
        .ingress
        .apply_operation_failure(&mut harness.session, deadline_failure)
        .expect("terminal deadline failure retries Stop");
    assert_eq!(final_retry.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(final_retry.drain, None);

    let stop_ack = final_retry
        .transition
        .action
        .execute_source(&harness.session, &mut harness.source, &cancelled_budget)
        .expect("independent teardown budget")
        .expect("stop acknowledgement");
    assert_eq!(harness.source.stop_budget_cancelled, vec![false; 3]);
    assert_eq!(harness.source.stop_calls, 3);
    let completed = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(completed.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(completed.drain, None);
    assert_eq!(harness.ingress.queue_diagnostics().cancellation_drains, 1);
    assert_eq!(
        harness
            .ingress
            .apply_operation_failure(&mut harness.session, cancelled_failure),
        Err(ScreenCaptureError::InvalidSessionTransition)
    );
    assert_eq!(harness.source.stop_calls, 3);
}

#[test]
fn every_epoch_flush_atomically_resets_queue_cache_and_rejects_replay() {
    let mut harness = capturing_harness(1);
    let image = harness.source.cursor_image(1);
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        )
        .expect("image");
    let metadata = cursor_metadata(harness.session.target(), Some(1), false);
    let frame = harness.source.frame(1, 1, 100, Some(metadata));
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        )
        .expect("frame");

    let stopped = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Stop)
        .expect("stop flush");
    let flush = stopped
        .transition
        .action
        .flush()
        .expect("mandatory epoch transition");
    let drain = stopped.drain.expect("ingress drain");
    assert_eq!(drain.queue.frames, 1);
    assert_eq!(drain.cursor.images, 1);
    assert_eq!(harness.ingress.active_stream(), None);
    assert_eq!(
        harness.ingress.apply_epoch_transition(flush),
        Err(ScreenCaptureError::NonMonotonicCaptureEpoch)
    );
}

#[test]
fn epoch_transition_b_on_a_is_rejected_before_queue_or_cache_mutation() {
    let mut session_a = capturing_harness(1);
    let mut session_b = capturing_harness(2);
    let image = session_a.source.cursor_image(1);
    session_a
        .ingress
        .handle_source_event(
            &mut session_a.session,
            &mut session_a.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        )
        .expect("A cursor image");
    let metadata = cursor_metadata(session_a.session.target(), Some(1), false);
    let frame = session_a.source.frame(1, 1, 100, Some(metadata));
    session_a
        .ingress
        .handle_source_event(
            &mut session_a.session,
            &mut session_a.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        )
        .expect("A frame");
    let transition_b = session_b
        .ingress
        .apply_intent(&mut session_b.session, ScreenSessionIntent::Stop)
        .expect("B stop transition")
        .transition
        .action
        .flush()
        .expect("B epoch transition");
    let epoch_a = session_a.ingress.capture_epoch();
    let stream_a = session_a.ingress.active_stream();
    let queue_a = session_a.ingress.queue_diagnostics();
    let cursor_a = session_a.ingress.cursor_descriptor();

    assert_eq!(
        session_a.ingress.apply_epoch_transition(transition_b),
        Err(ScreenCaptureError::EpochTransitionOwnershipMismatch)
    );
    assert_eq!(session_a.ingress.capture_epoch(), epoch_a);
    assert_eq!(session_a.ingress.active_stream(), stream_a);
    assert_eq!(session_a.ingress.queue_diagnostics(), queue_a);
    assert_eq!(session_a.ingress.cursor_descriptor(), cursor_a);
    let popped = session_a
        .ingress
        .try_pop(&mut session_a.session, 1, &CancellationToken::new())
        .expect("A queue remains intact");
    let ScreenIngressPopOutcome::Frame(frame) = popped else {
        panic!("A frame must survive the rejected B handoff");
    };
    assert_eq!(frame.sequence(), 1);
}

#[test]
fn permission_revocation_flushes_stops_and_requires_fresh_preflight() {
    let mut harness = capturing_harness(1);
    let revoked = harness
        .apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            3,
            PermissionPreflight::Revoked(SettingsGuidance::OpenSystemSettings),
        )))
        .expect("revocation");
    assert_eq!(
        revoked.transition.to,
        ScreenCapturePhase::Suspended(ScreenSuspensionReason::PermissionBlocked)
    );
    assert!(matches!(
        revoked.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert_eq!(
        revoked.transition.action.control_command(),
        ScreenControlCommand::RunPermissionPreflight
    );
    assert!(revoked.drain.is_some());

    let granted = harness
        .apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            4,
            PermissionPreflight::Granted,
        )))
        .expect("asynchronous grant");
    assert_eq!(granted.transition.to, ScreenCapturePhase::AwaitingPreflight);
    assert_eq!(
        granted.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(
        harness.apply_source_transition(ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            4,
            PermissionPreflight::Granted,
        ))),
        Err(ScreenCaptureError::StaleControlEvent)
    );
}

#[test]
fn sleep_and_wake_retire_data_and_cannot_restart_before_preflight_and_stop_ack() {
    let mut harness = capturing_harness(1);
    let mut sleeping = harness
        .apply_source_transition(ScreenSourceEvent::Sleep(control_stamp(
            source_instance(1),
            3,
        )))
        .expect("sleep");
    assert_eq!(
        sleeping.transition.to,
        ScreenCapturePhase::Suspended(ScreenSuspensionReason::Sleeping)
    );
    assert!(sleeping.drain.is_some());
    let wake = harness
        .apply_source_transition(ScreenSourceEvent::Wake(control_stamp(
            source_instance(1),
            4,
        )))
        .expect("wake");
    assert_eq!(wake.transition.to, ScreenCapturePhase::AwaitingPreflight);
    assert_eq!(
        wake.transition.action.control_command(),
        ScreenControlCommand::RunPermissionPreflight
    );
    harness.source.set_preflight_observation(observation(
        source_instance(1),
        5,
        PermissionPreflight::Granted,
    ));
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let preflight = harness
        .ingress
        .execute_control_action(
            &mut harness.session,
            &wake.transition.action,
            &mut harness.source,
            &budget,
        )
        .expect("fresh preflight");
    assert_eq!(preflight.transition.to, ScreenCapturePhase::Stopping);
    assert_eq!(
        preflight.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    let stop_ack = execute(
        &harness.session,
        &mut sleeping.transition.action,
        &mut harness.source,
    )
    .expect("stop")
    .expect("stop ack");
    let restart = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("stop ack");
    assert_eq!(restart.transition.to, ScreenCapturePhase::Starting);
    assert!(matches!(
        restart.transition.action.source_command(),
        ScreenSourceCommand::Start { .. }
    ));
}

#[test]
fn protected_content_detection_retires_stream_and_clear_waits_for_stop_ack() {
    let mut harness = capturing_harness(1);
    let mut detected = harness
        .apply_source_transition(ScreenSourceEvent::ProtectedContentDetected(control_stamp(
            source_instance(1),
            3,
        )))
        .expect("protected content");
    assert_eq!(
        detected.transition.to,
        ScreenCapturePhase::Suspended(ScreenSuspensionReason::ProtectedContent)
    );
    let cleared = harness
        .apply_source_transition(ScreenSourceEvent::ProtectedContentCleared(control_stamp(
            source_instance(1),
            4,
        )))
        .expect("clear");
    assert_eq!(cleared.transition.to, ScreenCapturePhase::Stopping);
    assert_eq!(
        cleared.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    let stop_ack = execute(
        &harness.session,
        &mut detected.transition.action,
        &mut harness.source,
    )
    .expect("stop")
    .expect("stop ack");
    let restarted = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("stop ack");
    assert_eq!(restarted.transition.to, ScreenCapturePhase::Starting);
}

#[test]
fn fail_closed_access_revocation_is_terminal_without_preflight_action() {
    let mut harness = ready_harness_with(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
        1,
    );
    start_and_ack(&mut harness);
    let source = source_instance(1);
    let capabilities = capabilities(source, 2, 2);
    let catalog = ScreenTargetSnapshot::new(source, 2, vec![window_target(source, 2, 1, 2)])
        .expect("catalog without lost target");
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    let removal = ScreenTargetChange::removed(
        ScreenTopologyStamp::new(source, 2, 1).expect("stamp"),
        harness.session.target().binding(),
        TargetLossReason::AccessRevoked,
        capabilities,
        catalog,
    )
    .expect("removal");
    let failed = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(removal)))
        .expect("fail closed");
    assert_eq!(
        failed.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::TargetLost)
    );
    assert!(matches!(
        failed.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert_eq!(
        failed.transition.action.control_command(),
        ScreenControlCommand::None
    );
}

#[test]
fn selected_target_loss_and_restore_wait_for_original_stop_before_fresh_start() {
    let mut harness = capturing_harness(1);
    let source = source_instance(1);
    let lost_capabilities = capabilities(source, 2, 2);
    let lost_catalog = ScreenTargetSnapshot::new(source, 2, vec![window_target(source, 2, 1, 2)])
        .expect("lost catalog");
    harness
        .source
        .update_topology(lost_capabilities.clone(), lost_catalog.clone());
    let removal = ScreenTargetChange::removed(
        ScreenTopologyStamp::new(source, 2, 1).expect("remove stamp"),
        harness.session.target().binding(),
        TargetLossReason::DisplayDisconnected,
        lost_capabilities,
        lost_catalog,
    )
    .expect("removal");
    let mut lost = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(removal)))
        .expect("target loss");
    assert_eq!(
        lost.transition.to,
        ScreenCapturePhase::Suspended(ScreenSuspensionReason::TargetLost)
    );

    let (restored_capabilities, restored_catalog, restored_target) = topology(3, 2, 2);
    harness
        .source
        .update_topology(restored_capabilities.clone(), restored_catalog.clone());
    let addition = ScreenTargetChange::added(
        ScreenTopologyStamp::new(source, 3, 1).expect("restore stamp"),
        restored_target,
        restored_capabilities,
        restored_catalog,
    )
    .expect("addition");
    let restored = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(addition)))
        .expect("restore");
    assert_eq!(restored.transition.to, ScreenCapturePhase::Stopping);
    assert_eq!(
        restored.transition.action.source_command(),
        ScreenSourceCommand::None
    );

    let stop_ack = execute(
        &harness.session,
        &mut lost.transition.action,
        &mut harness.source,
    )
    .expect("old stream stop")
    .expect("stop ack");
    let start = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("stop ack");
    assert_eq!(start.transition.to, ScreenCapturePhase::Starting);
    let ScreenSourceCommand::Start { stream, .. } = start.transition.action.source_command() else {
        panic!("expected fresh start");
    };
    assert_eq!(stream.target(), harness.session.target().binding());
    assert_eq!(stream.capture_epoch(), harness.session.capture_epoch());
}

#[test]
fn unrelated_hotplug_refreshes_catalog_without_retiring_the_active_stream() {
    let mut harness = capturing_harness(1);
    let old_stream = harness.ingress.active_stream().expect("active stream");
    let source = source_instance(1);
    let selected = display_target(source, 2, 1);
    let excluded = window_target(source, 2, 1, 2);
    let unrelated = window_target(source, 2, 1, 4);
    let catalog = ScreenTargetSnapshot::new(source, 2, vec![selected, excluded, unrelated.clone()])
        .expect("hotplug catalog");
    let capabilities = capabilities(source, 2, 2);
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    let event = ScreenTargetChange::added(
        ScreenTopologyStamp::new(source, 2, 1).expect("stamp"),
        unrelated,
        capabilities,
        catalog,
    )
    .expect("hotplug");
    let transition = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("unrelated hotplug");
    assert_eq!(transition.transition.to, ScreenCapturePhase::Capturing);
    assert_eq!(transition.transition.action.flush(), None);
    assert_eq!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::None
    );
    assert_eq!(harness.ingress.active_stream(), Some(old_stream));
    let frame = harness.source.frame_for(old_stream, 1, 1, 100, None);
    assert!(matches!(
        harness
            .ingress
            .handle_source_event(
                &mut harness.session,
                &mut harness.source,
                ScreenSourceEvent::Frame(frame),
                1,
                &CancellationToken::new(),
            )
            .expect("old active stream remains current"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
    ));
}

#[test]
fn selected_epoch_change_hidden_inside_unrelated_hotplug_forces_reconfigure() {
    let mut harness = capturing_harness(1);
    let event = unrelated_hotplug_event(&mut harness, 2, 1);
    let transition = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("disguised selected change");
    assert_eq!(transition.transition.to, ScreenCapturePhase::Reconfiguring);
    assert!(matches!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::Reconfigure { .. }
    ));
    assert!(transition.drain.is_some());
    assert_eq!(harness.ingress.active_stream(), None);
    assert_eq!(harness.session.target().target_epoch().get(), 2);
}

#[test]
fn excluded_epoch_change_hidden_inside_unrelated_hotplug_forces_reconfigure() {
    let mut harness = capturing_harness(1);
    let event = unrelated_hotplug_event(&mut harness, 1, 2);
    let transition = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("disguised exclusion change");
    assert_eq!(transition.transition.to, ScreenCapturePhase::Reconfiguring);
    assert!(matches!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::Reconfigure { .. }
    ));
    assert!(transition.drain.is_some());
    assert_eq!(
        harness.session.negotiated().request().excluded_windows()[0]
            .target_epoch()
            .get(),
        2
    );
}

#[test]
fn capability_loss_in_authentic_topology_fails_closed_and_retires_old_data() {
    let mut harness = capturing_harness(1);
    let old_stream = harness.ingress.active_stream().expect("active stream");
    let image = harness.source.cursor_image(1);
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::CursorImage(image),
            1,
            &CancellationToken::new(),
        )
        .expect("cursor image");
    let metadata = cursor_metadata(harness.session.target(), Some(1), false);
    let frame = harness.source.frame(1, 1, 100, Some(metadata));
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        )
        .expect("queued frame");

    let source = source_instance(1);
    let selected = display_target(source, 2, 1);
    let excluded = window_target(source, 2, 1, 2);
    let unrelated = window_target(source, 2, 1, 4);
    let catalog = ScreenTargetSnapshot::new(source, 2, vec![selected, excluded, unrelated.clone()])
        .expect("capability-loss catalog");
    let mut spec = capability_spec(source, 2, 2);
    spec.window_exclusion = false;
    spec.max_excluded_windows = 0;
    let capabilities = ScreenSourceCapabilities::new(spec).expect("reduced capabilities");
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    let event = ScreenTargetChange::added(
        ScreenTopologyStamp::new(source, 2, 1).expect("topology stamp"),
        unrelated,
        capabilities,
        catalog,
    )
    .expect("authentic capability-loss event");
    let mut failed = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("capability loss must fail closed");
    assert_eq!(
        failed.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::ContractInvalidated)
    );
    let drain = failed.drain.expect("atomic capability-loss drain");
    assert_eq!(drain.queue.frames, 1);
    assert_eq!(drain.queue.bytes, 100);
    assert_eq!(drain.cursor.images, 1);
    assert_eq!(drain.cursor.bytes, 64);
    assert!(matches!(
        failed.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    let late = harness
        .source
        .frame_for(old_stream, 2, 2, 100, Some(metadata));
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(late),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );

    let stop_ack = execute(
        &harness.session,
        &mut failed.transition.action,
        &mut harness.source,
    )
    .expect("capability-loss stop")
    .expect("stop acknowledgement");
    let completed = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(completed.transition.to, failed.transition.to);
    assert_eq!(completed.drain, None);
    assert_eq!(harness.source.active_stream, None);
    assert_eq!(harness.source.stop_calls, 1);
}

#[test]
fn removed_promised_exclusion_fails_closed_and_retires_old_data() {
    let mut harness = capturing_harness(1);
    let old_stream = harness.ingress.active_stream().expect("active stream");
    let frame = harness.source.frame(1, 1, 80, None);
    harness
        .ingress
        .handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(frame),
            1,
            &CancellationToken::new(),
        )
        .expect("queued frame");

    let source = source_instance(1);
    let removed_exclusion = harness.session.negotiated().request().excluded_windows()[0];
    let selected = display_target(source, 2, 1);
    let unrelated = window_target(source, 2, 1, 4);
    let catalog = ScreenTargetSnapshot::new(source, 2, vec![selected, unrelated])
        .expect("catalog without promised exclusion");
    let capabilities = capabilities(source, 2, 2);
    harness
        .source
        .update_topology(capabilities.clone(), catalog.clone());
    let event = ScreenTargetChange::removed(
        ScreenTopologyStamp::new(source, 2, 1).expect("topology stamp"),
        removed_exclusion,
        TargetLossReason::WindowClosed,
        capabilities,
        catalog,
    )
    .expect("excluded-window removal");
    let mut failed = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("lost exclusion must fail closed");
    assert_eq!(
        failed.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::ContractInvalidated)
    );
    let drain = failed.drain.expect("atomic exclusion-loss drain");
    assert_eq!(drain.queue.frames, 1);
    assert_eq!(drain.queue.bytes, 80);
    assert_eq!(drain.cursor.images, 0);
    assert!(matches!(
        failed.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    let late = harness.source.frame_for(old_stream, 2, 2, 80, None);
    assert_eq!(
        harness.ingress.handle_source_event(
            &mut harness.session,
            &mut harness.source,
            ScreenSourceEvent::Frame(late),
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::UnexpectedSourceData)
    );
    let stop_ack = execute(
        &harness.session,
        &mut failed.transition.action,
        &mut harness.source,
    )
    .expect("exclusion-loss stop")
    .expect("stop acknowledgement");
    harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(harness.source.active_stream, None);
    assert_eq!(harness.source.stop_calls, 1);
}

#[test]
fn unrelated_hotplug_supersedes_an_unexecuted_start_before_reissuing() {
    let mut harness = ready_harness(1);
    let mut held_start = harness
        .ingress
        .apply_intent(&mut harness.session, ScreenSessionIntent::Start)
        .expect("unexecuted start");
    let event = unrelated_hotplug_event(&mut harness, 1, 1);
    let mut superseded = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("unrelated hotplug");
    assert_eq!(superseded.transition.to, ScreenCapturePhase::Stopping);
    assert!(matches!(
        superseded.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert_eq!(
        execute(
            &harness.session,
            &mut held_start.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::StaleOperationAction)
    );
    let stop_ack = execute(
        &harness.session,
        &mut superseded.transition.action,
        &mut harness.source,
    )
    .expect("superseding stop")
    .expect("stop ack");
    let restarted = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("fresh start issued");
    assert_eq!(restarted.transition.to, ScreenCapturePhase::Starting);
    assert!(matches!(
        restarted.transition.action.source_command(),
        ScreenSourceCommand::Start { .. }
    ));
}

#[test]
fn source_bound_to_a_rejects_b_before_control_or_operation_native_calls() {
    let mut session_a = capturing_harness(1);
    let records_before = session_a.source.records.len();
    let enumeration_before = session_a.source.enumeration_calls;
    let preflight_before = session_a.source.preflight_calls;
    let binds_before = session_a.source.bind_calls;

    let (capabilities_b, catalog_b, negotiated_b) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source_b =
        BoundScreenCaptureSource::new(DummySource::new(capabilities_b, catalog_b), session_id(2))
            .expect("bound session B source");
    let mut session_b =
        ScreenCaptureSession::new(negotiated_b, source_b.binding()).expect("bound session B");
    let mut ingress_b = TestIngress::new(&session_b).expect("session B ingress");
    let preflight_b = session_b.initial_action();
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    assert_eq!(
        ingress_b.execute_control_action(
            &mut session_b,
            &preflight_b,
            &mut session_a.source,
            &budget,
        ),
        Err(ScreenControlExecutionError::Contract(
            ScreenCaptureError::SourceSessionOwnershipMismatch
        ))
    );
    assert_eq!(session_b.phase(), ScreenCapturePhase::AwaitingPreflight);

    ingress_b
        .execute_control_action(&mut session_b, &preflight_b, &mut source_b, &budget)
        .expect("bound session B preflight");
    let mut start_b = ingress_b
        .apply_intent(&mut session_b, ScreenSessionIntent::Start)
        .expect("session B start action");
    assert_eq!(
        start_b
            .transition
            .action
            .execute_source(&session_b, &mut session_a.source, &budget,),
        Err(ScreenOperationExecutionError::Contract(
            ScreenCaptureError::SourceSessionOwnershipMismatch
        ))
    );
    assert_eq!(session_a.source.records.len(), records_before);
    assert_eq!(session_a.source.enumeration_calls, enumeration_before);
    assert_eq!(session_a.source.preflight_calls, preflight_before);
    assert_eq!(session_a.source.bind_calls, binds_before);
    assert!(session_a.source.active_stream.is_some());
}

#[test]
fn control_action_b_on_a_is_rejected_before_adapter_or_session_side_effects() {
    let (capabilities_a, catalog_a, negotiated_a) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source_a =
        BoundScreenCaptureSource::new(DummySource::new(capabilities_a, catalog_a), session_id(1))
            .expect("bound A source");
    let mut session_a =
        ScreenCaptureSession::new(negotiated_a, source_a.binding()).expect("session A");
    let mut ingress_a = TestIngress::new(&session_a).expect("ingress A");

    let (capabilities_b, catalog_b, negotiated_b) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source_b =
        BoundScreenCaptureSource::new(DummySource::new(capabilities_b, catalog_b), session_id(2))
            .expect("bound B source");
    let mut session_b =
        ScreenCaptureSession::new(negotiated_b, source_b.binding()).expect("session B");
    let mut ingress_b = TestIngress::new(&session_b).expect("ingress B");
    let action_b = session_b.initial_action();
    let phase_a = session_a.phase();
    let diagnostics_a = session_a.diagnostics();
    let queue_a = ingress_a.queue_diagnostics();
    let preflight_calls_a = source_a.preflight_calls;
    let enumeration_calls_a = source_a.enumeration_calls;
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");

    assert_eq!(
        ingress_a.execute_control_action(&mut session_a, &action_b, &mut source_a, &budget),
        Err(ScreenControlExecutionError::Contract(
            ScreenCaptureError::ActionSessionOwnershipMismatch,
        ))
    );
    assert_eq!(session_a.phase(), phase_a);
    assert_eq!(session_a.diagnostics(), diagnostics_a);
    assert_eq!(ingress_a.queue_diagnostics(), queue_a);
    assert_eq!(source_a.preflight_calls, preflight_calls_a);
    assert_eq!(source_a.enumeration_calls, enumeration_calls_a);

    let ready_b = ingress_b
        .execute_control_action(&mut session_b, &action_b, &mut source_b, &budget)
        .expect("B action remains valid and retryable for B");
    assert_eq!(ready_b.transition.to, ScreenCapturePhase::Ready);
    assert_eq!(source_b.preflight_calls, 1);
}

#[test]
fn session_scoped_stop_quiesces_only_a_during_unexecuted_reconfigure() {
    let (mut harness, mut session_b) = shared_backend_capturing_harnesses();
    let old_native_stream = harness.source.active_stream.expect("old native stream");
    let binding_a = harness.source.session_binding.expect("session A binding");
    let binding_b = session_b.source.session_binding.expect("session B binding");
    let stream_b = session_b.source.active_stream.expect("session B stream");
    let event = reconfigure_event(&mut harness, 2, 2, 1);
    let mut reconfigure = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("pending reconfigure");
    let ScreenSourceCommand::Reconfigure {
        stream: pending_stream,
        ..
    } = reconfigure.transition.action.source_command()
    else {
        panic!("expected reconfigure");
    };
    assert_ne!(pending_stream, old_native_stream);

    let mut cancelled = harness
        .ingress
        .cancel_session(&mut harness.session)
        .expect("cancel pending reconfigure");
    let ScreenSourceCommand::Stop {
        stream: invalidated_stream,
        ..
    } = cancelled.transition.action.source_command()
    else {
        panic!("expected session-scoped stop");
    };
    assert_eq!(invalidated_stream, pending_stream);
    let ack = execute(
        &harness.session,
        &mut cancelled.transition.action,
        &mut harness.source,
    )
    .expect("session-scoped stop")
    .expect("stop acknowledgement");
    assert_eq!(
        harness.source.stopped_native_streams.last(),
        Some(&Some(old_native_stream))
    );
    assert_eq!(harness.source.active_stream, None);
    assert_eq!(harness.source.native_stream(binding_a), None);
    assert_eq!(harness.source.native_stream(binding_b), Some(stream_b));
    assert_eq!(session_b.source.active_stream, Some(stream_b));
    let stop_record = harness.source.records.last().expect("stop record");
    assert_eq!(stop_record.session_binding, binding_a);
    assert_eq!(stop_record.predecessor_stream, Some(old_native_stream));
    let frame_b = session_b.source.frame_for(stream_b, 1, 1, 50, None);
    assert!(matches!(
        session_b
            .ingress
            .handle_source_event(
                &mut session_b.session,
                &mut session_b.source,
                ScreenSourceEvent::Frame(frame_b),
                1,
                &CancellationToken::new(),
            )
            .expect("session B remains live"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
    ));
    harness
        .ingress
        .complete_operation(&mut harness.session, ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(
        execute(
            &harness.session,
            &mut reconfigure.transition.action,
            &mut harness.source,
        ),
        Err(ScreenOperationExecutionError::StaleOperationAction)
    );
}

#[test]
fn session_scoped_stop_quiesces_only_a_after_dispatched_reconfigure() {
    let (mut harness, mut session_b) = shared_backend_capturing_harnesses();
    let binding_a = harness.source.session_binding.expect("session A binding");
    let binding_b = session_b.source.session_binding.expect("session B binding");
    let stream_b = session_b.source.active_stream.expect("session B stream");
    let old_native_stream = harness.source.active_stream.expect("old session A stream");
    let event = reconfigure_event(&mut harness, 2, 2, 1);
    let mut reconfigure = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(event)))
        .expect("pending reconfigure");
    let reconfigure_ack = execute(
        &harness.session,
        &mut reconfigure.transition.action,
        &mut harness.source,
    )
    .expect("native reconfigure dispatch")
    .expect("held reconfigure acknowledgement");
    let dispatched_stream = reconfigure_ack.stream();
    assert_eq!(harness.source.active_stream, Some(dispatched_stream));

    let mut cancelled = harness
        .ingress
        .cancel_session(&mut harness.session)
        .expect("cancel dispatched reconfigure");
    let stop_ack = execute(
        &harness.session,
        &mut cancelled.transition.action,
        &mut harness.source,
    )
    .expect("session-scoped stop")
    .expect("stop acknowledgement");
    assert_eq!(
        harness.source.stopped_native_streams.last(),
        Some(&Some(dispatched_stream))
    );
    assert_eq!(harness.source.active_stream, None);
    assert_eq!(harness.source.native_stream(binding_a), None);
    assert_eq!(harness.source.native_stream(binding_b), Some(stream_b));
    assert_eq!(session_b.source.active_stream, Some(stream_b));
    let stop_record = harness.source.records.last().expect("stop record");
    assert_eq!(stop_record.session_binding, binding_a);
    assert_eq!(stop_record.stream, dispatched_stream);
    assert_eq!(stop_record.predecessor_stream, Some(old_native_stream));
    let frame_b = session_b.source.frame_for(stream_b, 1, 1, 50, None);
    assert!(matches!(
        session_b
            .ingress
            .handle_source_event(
                &mut session_b.session,
                &mut session_b.source,
                ScreenSourceEvent::Frame(frame_b),
                1,
                &CancellationToken::new(),
            )
            .expect("session B remains live"),
        ScreenIngressOutcome::Frame(ScreenQueuePushOutcome::Accepted)
    ));
    assert_eq!(
        harness
            .ingress
            .complete_operation(&mut harness.session, reconfigure_ack),
        Err(ScreenCaptureError::InvalidSessionTransition)
    );
    let completed = harness
        .ingress
        .complete_operation(&mut harness.session, stop_ack)
        .expect("terminal stop acknowledgement");
    assert_eq!(completed.transition.to, ScreenCapturePhase::Cancelled);
    assert_eq!(completed.drain, None);
}

#[test]
fn stale_topology_is_rejected_without_partial_session_or_ingress_mutation() {
    let mut harness = capturing_harness(1);
    let change = reconfigure_event(&mut harness, 2, 2, 1);
    let transition = harness
        .apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(change)))
        .expect("first topology event");
    let epoch = harness.session.capture_epoch();
    let target = harness.session.target().binding();
    assert_eq!(transition.transition.to, ScreenCapturePhase::Reconfiguring);

    let (old_capabilities, old_catalog, old_target) = topology(1, 1, 2);
    let stale = ScreenTargetChange::reconfigured(
        ScreenTopologyStamp::new(source_instance(1), 1, 2).expect("stale stamp"),
        old_target,
        old_capabilities,
        old_catalog,
    )
    .expect("well-formed stale event");
    assert_eq!(
        harness.apply_source_transition(ScreenSourceEvent::TargetChanged(Box::new(stale))),
        Err(ScreenCaptureError::StaleTopologyEvent)
    );
    assert_eq!(harness.session.capture_epoch(), epoch);
    assert_eq!(harness.session.target().binding(), target);
    assert_eq!(harness.ingress.capture_epoch(), epoch);
}

#[test]
fn geometry_handles_negative_origins_fractional_scale_and_rotation() {
    let logical = LogicalRect::new(-300, -200, 4, 2).expect("logical");
    let transform = DisplayGeometryTransform::new(
        logical,
        PhysicalRect::new(10, 20, 3, 6).expect("physical"),
        DpiScale::new(3, 2).expect("fractional scale"),
        Rotation::Degrees90,
    )
    .expect("rotated transform");
    let mapped = transform
        .logical_rect_to_physical(LogicalRect::new(-299, -200, 2, 1).expect("selection"))
        .expect("mapped selection");
    assert!(mapped.width() > 0);
    assert!(mapped.height() > 0);
    assert!(transform.logical_point_to_physical(-300, -200).is_ok());
    assert_eq!(
        transform.logical_point_to_physical(0, 0),
        Err(ScreenCaptureError::GeometryOutsideDisplay)
    );
}

#[test]
fn capability_profiles_are_exact_tuples_not_an_invented_cross_product() {
    let source = source_instance(1);
    let selected = display_target(source, 1, 1);
    let catalog = ScreenTargetSnapshot::new(source, 1, vec![selected.clone()]).expect("catalog");
    let mut spec = capability_spec(source, 1, 1);
    spec.frame_profiles = vec![
        ScreenFrameProfile {
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            memory: FrameMemory::Cpu,
            max_width: 100,
            max_height: 100,
            max_frames_per_second: 120,
        },
        ScreenFrameProfile {
            pixel_format: PixelFormat::Nv12,
            color_space: ColorSpace::Bt709Limited,
            memory: FrameMemory::Cpu,
            max_width: 3_840,
            max_height: 2_160,
            max_frames_per_second: 120,
        },
    ];
    let capabilities = ScreenSourceCapabilities::new(spec).expect("capabilities");
    let request = request(
        selected,
        vec![],
        cursor_policy(CursorCaptureMode::Metadata, false, false),
        default_queue(),
        TargetRecoveryPolicy::FailClosed,
        ProtectedContentPolicy::SuspendUntilClear,
    );
    assert_eq!(
        negotiate_screen_capture(&capabilities, &catalog, request),
        Err(ScreenCaptureError::UnsupportedFrameSpec)
    );
}

#[test]
fn operation_budget_is_cooperatively_cancelled_and_bounded() {
    let cancellation = CancellationToken::new();
    assert!(ScreenOperationBudget::new(&cancellation, Duration::from_secs(31)).is_err());
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    cancellation.cancel();
    assert_eq!(
        budget.check(),
        Err(ScreenSourceFailure::new(
            ScreenSourceFailureCode::Cancelled,
            false
        ))
    );
}

#[test]
fn preflight_and_permission_request_errors_leave_control_actions_retryable() {
    let (capabilities, catalog, negotiated) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source =
        BoundScreenCaptureSource::new(DummySource::new(capabilities, catalog), session_id(1))
            .expect("bound source");
    let mut session =
        ScreenCaptureSession::new(negotiated, source.binding()).expect("bound session");
    let mut ingress = TestIngress::new(&session).expect("ingress");
    let preflight = session.initial_action();
    source.set_preflight_failure(ScreenSourceFailureCode::AdapterUnavailable);
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    assert!(matches!(
        ingress.execute_control_action(&mut session, &preflight, &mut source, &budget),
        Err(ScreenControlExecutionError::Source(_))
    ));
    assert_eq!(session.phase(), ScreenCapturePhase::AwaitingPreflight);
    let ready = ingress
        .execute_control_action(&mut session, &preflight, &mut source, &budget)
        .expect("preflight retry");
    assert_eq!(ready.transition.to, ScreenCapturePhase::Ready);

    let (capabilities, catalog, negotiated) = initial_contract(
        cursor_policy(CursorCaptureMode::Metadata, true, true),
        default_queue(),
        TargetRecoveryPolicy::ResumeSameTarget { max_attempts: 2 },
        ProtectedContentPolicy::SuspendUntilClear,
    );
    let mut source =
        BoundScreenCaptureSource::new(DummySource::new(capabilities, catalog), session_id(2))
            .expect("bound source");
    let mut session =
        ScreenCaptureSession::new(negotiated, source.binding()).expect("bound session");
    let mut ingress = TestIngress::new(&session).expect("ingress");
    source.set_preflight_observation(observation(
        source_instance(1),
        2,
        PermissionPreflight::PromptRequired,
    ));
    let prompt_action = session.initial_action();
    ingress
        .execute_control_action(&mut session, &prompt_action, &mut source, &budget)
        .expect("prompt required");
    let permission = ingress
        .apply_intent(&mut session, ScreenSessionIntent::RequestPermission)
        .expect("request permission");
    source.set_request_failure(ScreenSourceFailureCode::NativeOperationFailed);
    assert!(matches!(
        ingress.execute_control_action(
            &mut session,
            &permission.transition.action,
            &mut source,
            &budget,
        ),
        Err(ScreenControlExecutionError::Source(_))
    ));
    assert_eq!(
        session.phase(),
        ScreenCapturePhase::AwaitingPermissionResult
    );
    let granted = ingress
        .execute_control_action(
            &mut session,
            &permission.transition.action,
            &mut source,
            &budget,
        )
        .expect("permission retry");
    assert_eq!(granted.transition.to, ScreenCapturePhase::Ready);
}

#[test]
fn raw_poll_failure_is_bound_to_the_active_stream_and_returns_exact_stop() {
    let mut harness = capturing_harness(1);
    harness
        .source
        .set_poll_failure(ScreenSourceFailureCode::AdapterUnavailable);
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let outcome = harness
        .ingress
        .poll_source(
            &mut harness.session,
            &mut harness.source,
            &budget,
            0,
            &cancellation,
        )
        .expect("normalized poll failure")
        .expect("failure transition");
    let ScreenIngressOutcome::Session(transition) = outcome else {
        panic!("expected session transition");
    };
    assert!(matches!(
        transition.transition.to,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::Source(
            ScreenSourceFailureCode::AdapterUnavailable
        ))
    ));
    assert!(matches!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert!(transition.drain.is_some());
}

#[test]
fn raw_poll_cancellation_uses_the_single_terminal_cancel_path() {
    let mut harness = capturing_harness(1);
    harness
        .source
        .set_poll_failure(ScreenSourceFailureCode::Cancelled);
    let cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&cancellation, Duration::from_secs(1)).expect("budget");
    let outcome = harness
        .ingress
        .poll_source(
            &mut harness.session,
            &mut harness.source,
            &budget,
            0,
            &cancellation,
        )
        .expect("normalized cancellation")
        .expect("cancel transition");
    let ScreenIngressOutcome::Session(transition) = outcome else {
        panic!("expected cancellation transition");
    };
    assert_eq!(transition.transition.to, ScreenCapturePhase::Cancelled);
    assert!(matches!(
        transition.transition.action.source_command(),
        ScreenSourceCommand::Stop { .. }
    ));
    assert!(transition.drain.is_some());
}

#[test]
fn b_permission_topology_and_sleep_envelopes_cannot_mutate_a() {
    let mut session_a = capturing_harness(1);
    let mut session_b = capturing_harness(2);
    let image_a = session_a.source.cursor_image(1);
    session_a
        .ingress
        .handle_source_event(
            &mut session_a.session,
            &mut session_a.source,
            ScreenSourceEvent::CursorImage(image_a),
            1,
            &CancellationToken::new(),
        )
        .expect("A cursor image");
    let frame_a = session_a.source.frame(1, 1, 100, None);
    session_a
        .ingress
        .handle_source_event(
            &mut session_a.session,
            &mut session_a.source,
            ScreenSourceEvent::Frame(frame_a),
            1,
            &CancellationToken::new(),
        )
        .expect("A frame");
    let topology_b = unrelated_hotplug_event(&mut session_b, 1, 1);
    let events_b = vec![
        ScreenSourceEvent::PermissionChanged(observation(
            source_instance(1),
            3,
            PermissionPreflight::Revoked(SettingsGuidance::OpenSystemSettings),
        )),
        ScreenSourceEvent::TargetChanged(Box::new(topology_b)),
        ScreenSourceEvent::Sleep(control_stamp(source_instance(1), 3)),
    ];
    let phase_a = session_a.session.phase();
    let diagnostics_a = session_a.session.diagnostics();
    let epoch_a = session_a.ingress.capture_epoch();
    let stream_a = session_a.ingress.active_stream();
    let queue_a = session_a.ingress.queue_diagnostics();
    let cursor_a = session_a.ingress.cursor_descriptor();
    let poll_cancellation = CancellationToken::new();
    let budget = ScreenOperationBudget::new(&poll_cancellation, Duration::from_secs(1))
        .expect("B poll budget");
    let cancelled_lifetime = CancellationToken::new();
    cancelled_lifetime.cancel();

    for event_b in events_b {
        session_b.source.queue_event(event_b);
        let envelope_b = session_b
            .source
            .poll_owned_event(&budget)
            .expect("B poll")
            .expect("B envelope");
        assert_eq!(
            session_a.ingress.apply_source_event(
                &mut session_a.session,
                envelope_b,
                2,
                &cancelled_lifetime,
            ),
            Err(ScreenCaptureError::SourceEventOwnershipMismatch)
        );
        assert_eq!(session_a.session.phase(), phase_a);
        assert_eq!(session_a.session.diagnostics(), diagnostics_a);
        assert_eq!(session_a.ingress.capture_epoch(), epoch_a);
        assert_eq!(session_a.ingress.active_stream(), stream_a);
        assert_eq!(session_a.ingress.queue_diagnostics(), queue_a);
        assert_eq!(session_a.ingress.cursor_descriptor(), cursor_a);
    }

    let popped = session_a
        .ingress
        .try_pop(&mut session_a.session, 2, &CancellationToken::new())
        .expect("A frame remains queued");
    let ScreenIngressPopOutcome::Frame(frame) = popped else {
        panic!("foreign envelopes must not drain A");
    };
    assert_eq!(frame.sequence(), 1);
}

#[test]
fn mixed_ingress_session_and_source_fail_before_poll_or_queue_side_effects() {
    let mut session_a = capturing_harness(1);
    let mut session_b = capturing_harness(2);
    let frame_a = session_a.source.frame(1, 1, 100, None);
    session_a
        .ingress
        .handle_source_event(
            &mut session_a.session,
            &mut session_a.source,
            ScreenSourceEvent::Frame(frame_a),
            1,
            &CancellationToken::new(),
        )
        .expect("queued session A frame");
    let queue_before = session_a.ingress.queue_diagnostics();
    let poll_calls_before = session_b.source.poll_calls;
    let phase_a = session_a.session.phase();
    let phase_b = session_b.session.phase();
    let budget_cancellation = CancellationToken::new();
    let budget =
        ScreenOperationBudget::new(&budget_cancellation, Duration::from_secs(1)).expect("budget");
    let cancelled_lifetime = CancellationToken::new();
    cancelled_lifetime.cancel();

    assert_eq!(
        session_a.ingress.poll_source(
            &mut session_b.session,
            &mut session_b.source,
            &budget,
            2,
            &cancelled_lifetime,
        ),
        Err(ScreenCaptureError::IngressSessionMismatch)
    );
    assert_eq!(session_b.source.poll_calls, poll_calls_before);
    assert_eq!(session_b.session.phase(), phase_b);
    assert_eq!(session_a.ingress.queue_diagnostics(), queue_before);
    assert!(matches!(
        session_a
            .ingress
            .try_pop(&mut session_b.session, 2, &cancelled_lifetime),
        Err(ScreenCaptureError::IngressSessionMismatch)
    ));
    assert_eq!(session_b.session.phase(), phase_b);
    assert_eq!(session_a.ingress.queue_diagnostics(), queue_before);

    assert_eq!(
        session_a.ingress.poll_source(
            &mut session_a.session,
            &mut session_b.source,
            &budget,
            2,
            &CancellationToken::new(),
        ),
        Err(ScreenCaptureError::SourceSessionOwnershipMismatch)
    );
    assert_eq!(session_b.source.poll_calls, poll_calls_before);
    let frame_b = session_b.source.frame(1, 1, 100, None);
    session_b
        .source
        .queue_event(ScreenSourceEvent::Frame(frame_b));
    let envelope_b = session_b
        .source
        .poll_owned_event(&budget)
        .expect("B event poll")
        .expect("B event envelope");
    assert_eq!(
        session_a.ingress.apply_source_event(
            &mut session_a.session,
            envelope_b,
            2,
            &cancelled_lifetime,
        ),
        Err(ScreenCaptureError::SourceEventOwnershipMismatch)
    );
    assert_eq!(session_a.ingress.queue_diagnostics(), queue_before);
    assert_eq!(session_a.session.phase(), phase_a);
    let popped = session_a
        .ingress
        .try_pop(&mut session_a.session, 2, &CancellationToken::new())
        .expect("valid session A pop");
    let ScreenIngressPopOutcome::Frame(frame) = popped else {
        panic!("mixed-session pop must not consume A's frame");
    };
    assert_eq!(frame.sequence(), 1);
}

#[test]
fn diagnostics_and_debug_output_remain_low_cardinality_and_redacted() {
    let harness = capturing_harness(1);
    let diagnostics = harness.session.diagnostics();
    assert_eq!(
        diagnostics.schema_version,
        SCREEN_CAPTURE_DIAGNOSTIC_VERSION
    );
    assert_eq!(diagnostics.phase, ScreenCapturePhase::Capturing);
    assert_eq!(diagnostics.target_kind, ScreenTargetKind::Display);
    let rendered = format!(
        "{:?}{:?}{:?}",
        harness.session,
        harness.source.record(0).stream,
        harness.session.target()
    );
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("window title"));
}
