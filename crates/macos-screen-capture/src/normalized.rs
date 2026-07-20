//! Provider-neutral ownership and event adapter for ScreenCaptureKit.

use std::collections::VecDeque;

use frame_media::{
    CursorCaptureMode, PermissionPreflight, PixelFormat, PlatformScreenSource,
    SCREEN_CAPTURE_CONTRACT_VERSION, ScreenCaptureError, ScreenCaptureSource, ScreenControlEpoch,
    ScreenControlStamp, ScreenCursorModes, ScreenFrame, ScreenFrameEnvelope, ScreenFrameProfile,
    ScreenOperationBudget, ScreenOperationKind, ScreenOperationTicket, ScreenPermissionObservation,
    ScreenSourceCallTicket, ScreenSourceCapabilities, ScreenSourceCapabilitySpec,
    ScreenSourceFailure, ScreenSourceFailureCode, ScreenSourceInstanceId, ScreenSourcePollResult,
    ScreenSourceSessionBinding, ScreenSourceSessionTicket, ScreenTargetKind, ScreenTargetKinds,
    ScreenTargetSnapshot,
};

use crate::{
    CALLBACK_QUEUE_CAPACITY, MAX_CAPTURE_HEIGHT, MAX_CAPTURE_WIDTH, MacOsCaptureConfig,
    MacOsCaptureError, MacOsCaptureFrame, MacOsRegionSelection, MacOsScreenCaptureSource,
};

/// ScreenCaptureKit source bound to the normalized capture session contract.
///
/// The wrapper owns a finite callback tail and never labels ScreenCaptureKit's
/// ambiguous blank/suspended status as exact protected content. Production
/// negotiation therefore uses the fail-session content-unavailable contract.
pub struct MacOsNormalizedScreenCaptureSource {
    source: MacOsScreenCaptureSource,
    snapshot: ScreenTargetSnapshot,
    regions: Vec<MacOsRegionSelection>,
    capabilities: ScreenSourceCapabilities,
    binding: Option<ScreenSourceSessionBinding>,
    active_stream: Option<frame_media::ScreenStreamStamp>,
    stopped_tail: VecDeque<ScreenFrame<Box<[u8]>>>,
    next_control_sequence: u64,
}

impl MacOsNormalizedScreenCaptureSource {
    pub fn new(
        source: MacOsScreenCaptureSource,
        snapshot: ScreenTargetSnapshot,
    ) -> Result<Self, MacOsCaptureError> {
        if source.source_instance() != snapshot.source_instance() {
            return Err(MacOsCaptureError::StaleOrForeignTarget);
        }
        let regions = snapshot
            .targets()
            .iter()
            .filter(|target| target.kind() == ScreenTargetKind::Region)
            .map(|target| {
                let display = target
                    .containing_display_binding()
                    .ok_or(MacOsCaptureError::InvalidRegionGeometry)?;
                MacOsRegionSelection::new(display, target.logical_bounds())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capabilities = capabilities(source.source_instance(), snapshot.generation())?;
        let next_control_sequence = capabilities
            .control_sequence()
            .checked_add(1)
            .ok_or(MacOsCaptureError::TopologyGenerationExhausted)?;
        Ok(Self {
            source,
            snapshot,
            regions,
            capabilities,
            binding: None,
            active_stream: None,
            stopped_tail: VecDeque::with_capacity(CALLBACK_QUEUE_CAPACITY),
            next_control_sequence,
        })
    }

    #[must_use]
    pub const fn raw_source(&self) -> &MacOsScreenCaptureSource {
        &self.source
    }

    fn validate_call(
        &self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        budget.check()?;
        if self.binding != Some(ticket.binding()) {
            return Err(ScreenSourceFailure::new(
                ScreenSourceFailureCode::NativeOperationFailed,
                false,
            ));
        }
        Ok(())
    }

    fn permission_observation(
        &mut self,
        permission: PermissionPreflight,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        let sequence = self.next_control_sequence;
        self.next_control_sequence = sequence.checked_add(1).ok_or_else(|| {
            ScreenSourceFailure::new(ScreenSourceFailureCode::NativeOperationFailed, false)
        })?;
        let stamp = ScreenControlStamp::new(
            self.source.source_instance(),
            self.capabilities.control_epoch(),
            sequence,
        )
        .map_err(|_| {
            ScreenSourceFailure::new(ScreenSourceFailureCode::NativeOperationFailed, false)
        })?;
        Ok(ScreenPermissionObservation { stamp, permission })
    }

    fn update_catalog(
        &mut self,
        snapshot: ScreenTargetSnapshot,
    ) -> Result<ScreenTargetSnapshot, ScreenSourceFailure> {
        self.capabilities = capabilities(self.source.source_instance(), snapshot.generation())
            .map_err(source_failure)?;
        self.snapshot = snapshot;
        Ok(self.snapshot.clone())
    }

    fn validate_operation(
        &self,
        ticket: &ScreenOperationTicket,
        expected_kind: ScreenOperationKind,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        budget.check()?;
        if self.binding != Some(ticket.session_binding()) || ticket.kind() != expected_kind {
            return Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false));
        }
        Ok(())
    }
}

impl ScreenCaptureSource for MacOsNormalizedScreenCaptureSource {
    type FramePayload = Box<[u8]>;
    type CursorImagePayload = Box<[u8]>;

    fn source_instance(&self) -> ScreenSourceInstanceId {
        self.source.source_instance()
    }

    fn session_binding(&self) -> Option<ScreenSourceSessionBinding> {
        self.binding
    }

    fn bind_session(
        &mut self,
        ticket: ScreenSourceSessionTicket,
    ) -> Result<(), ScreenCaptureError> {
        match self.binding {
            Some(binding) if binding != ticket.binding() => {
                Err(ScreenCaptureError::SourceSessionOwnershipMismatch)
            }
            Some(_) => Ok(()),
            None => {
                self.binding = Some(ticket.binding());
                Ok(())
            }
        }
    }

    fn capabilities<'a>(
        &'a self,
        ticket: &ScreenSourceCallTicket<'_>,
    ) -> &'a ScreenSourceCapabilities {
        debug_assert_eq!(self.binding, Some(ticket.binding()));
        &self.capabilities
    }

    fn preflight(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        self.validate_call(ticket, budget)?;
        let permission = self.source.preflight_permission();
        self.permission_observation(permission)
    }

    fn request_permission(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        self.validate_call(ticket, budget)?;
        let permission = self.source.request_permission();
        self.permission_observation(permission)
    }

    fn enumerate_targets(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenTargetSnapshot, ScreenSourceFailure> {
        self.validate_call(ticket, budget)?;
        let regions = self.regions.clone();
        let snapshot = self
            .source
            .enumerate_targets(&regions)
            .map_err(source_failure)?;
        self.update_catalog(snapshot)
    }

    fn start(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.validate_operation(&ticket, ScreenOperationKind::Start, budget)?;
        if self.active_stream.is_some()
            || !self.stopped_tail.is_empty()
            || ticket.catalog_generation() != self.snapshot.generation()
            || ticket.negotiated().catalog() != &self.snapshot
        {
            return Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false));
        }
        let request = ticket.negotiated().request();
        let config = MacOsCaptureConfig::new(
            request.target().binding(),
            request.output(),
            request.cursor().mode(),
        )
        .map_err(|error| ticket.failure(source_failure_code(error).0, false))?;
        self.source.start(config).map_err(|error| {
            let (code, retryable) = source_failure_code(error);
            ticket.failure(code, retryable)
        })?;
        self.active_stream = Some(ticket.stream());
        Ok(())
    }

    fn reconfigure(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.validate_operation(&ticket, ScreenOperationKind::Reconfigure, budget)?;
        Err(ticket.failure(ScreenSourceFailureCode::AdapterUnavailable, false))
    }

    fn poll_event(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> ScreenSourcePollResult<Self::FramePayload, Self::CursorImagePayload> {
        self.validate_call(ticket, budget)?;
        let stream = self.active_stream.ok_or_else(|| {
            ScreenSourceFailure::new(ScreenSourceFailureCode::NativeOperationFailed, false)
        })?;
        self.source
            .poll_frame()
            .map_err(source_failure)?
            .map(|frame| normalized_frame(stream, frame))
            .transpose()
            .map(|frame| frame.map(frame_media::ScreenSourceEvent::Frame))
            .map_err(source_failure)
    }

    fn poll_stopped_event(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> ScreenSourcePollResult<Self::FramePayload, Self::CursorImagePayload> {
        self.validate_call(ticket, budget)?;
        Ok(self
            .stopped_tail
            .pop_front()
            .map(frame_media::ScreenSourceEvent::Frame))
    }

    fn stop(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.validate_operation(&ticket, ScreenOperationKind::Stop, budget)?;
        let Some(stream) = self.active_stream else {
            return if self.source.is_running() {
                Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false))
            } else {
                Ok(())
            };
        };
        if ticket.stream() != stream && ticket.predecessor_stream() != Some(stream) {
            return Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false));
        }
        let frames = self.source.stop_and_drain_frames().map_err(|error| {
            let (code, retryable) = source_failure_code(error.into_capture_error());
            ticket.failure(code, retryable)
        })?;
        // Native stop is authoritative even if one retained callback frame is
        // malformed. Clear the active stream before converting the finite tail
        // so a conversion failure cannot make a stopped source look live.
        self.active_stream = None;
        if frames.len() > CALLBACK_QUEUE_CAPACITY {
            return Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false));
        }
        let tail = frames
            .into_iter()
            .map(|frame| normalized_frame(stream, frame))
            .collect::<Result<VecDeque<_>, _>>()
            .map_err(|error| {
                let (code, retryable) = source_failure_code(error);
                ticket.failure(code, retryable)
            })?;
        self.stopped_tail = tail;
        Ok(())
    }
}

fn capabilities(
    source_instance: ScreenSourceInstanceId,
    topology_generation: u64,
) -> Result<ScreenSourceCapabilities, MacOsCaptureError> {
    let spec = ScreenSourceCapabilitySpec {
        contract_version: SCREEN_CAPTURE_CONTRACT_VERSION,
        source: PlatformScreenSource::ScreenCaptureKit,
        source_instance,
        topology_generation,
        control_epoch: ScreenControlEpoch::new(1)
            .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?,
        control_sequence: 1,
        targets: ScreenTargetKinds::none()
            .with(ScreenTargetKind::Display)
            .with(ScreenTargetKind::Window)
            .with(ScreenTargetKind::Region),
        cursor_modes: ScreenCursorModes::none()
            .with(CursorCaptureMode::Hidden)
            .with(CursorCaptureMode::EmbeddedInFrame),
        cursor_image_metadata: false,
        cursor_click_metadata: false,
        cursor_desktop_logical_coordinates: false,
        cursor_frame_physical_coordinates: false,
        frame_profiles: vec![ScreenFrameProfile {
            pixel_format: PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            memory: frame_media::FrameMemory::Cpu,
            max_width: MAX_CAPTURE_WIDTH,
            max_height: MAX_CAPTURE_HEIGHT,
            max_frames_per_second: 60,
        }],
        permission_preflight: true,
        topology_events: false,
        target_recovery: false,
        protected_content_events: false,
        content_unavailable_failures: true,
        window_exclusion: false,
        max_excluded_windows: 0,
        bounded_appsrc_ingress: true,
    };
    ScreenSourceCapabilities::new(spec).map_err(|_| MacOsCaptureError::MediaCatalogRejected)
}

fn normalized_frame(
    stream: frame_media::ScreenStreamStamp,
    frame: MacOsCaptureFrame,
) -> Result<ScreenFrame<Box<[u8]>>, MacOsCaptureError> {
    let sequence = frame.sequence();
    let timestamp = frame.timestamp();
    let spec = frame.spec();
    let payload = frame.into_pixels().into_boxed_slice();
    let retained_bytes =
        u64::try_from(payload.len()).map_err(|_| MacOsCaptureError::FrameAllocationExceedsLimit)?;
    ScreenFrame::new(
        ScreenFrameEnvelope {
            stream,
            sequence,
            timestamp,
            spec,
            retained_bytes,
            cursor: None,
        },
        payload,
    )
    .map_err(|_| MacOsCaptureError::InvalidSampleBuffer)
}

fn source_failure(error: MacOsCaptureError) -> ScreenSourceFailure {
    let (code, retryable) = source_failure_code(error);
    ScreenSourceFailure::new(code, retryable)
}

const fn source_failure_code(error: MacOsCaptureError) -> (ScreenSourceFailureCode, bool) {
    match error {
        MacOsCaptureError::PermissionDenied => (ScreenSourceFailureCode::PermissionDenied, true),
        MacOsCaptureError::TargetNoLongerAvailable
        | MacOsCaptureError::StaleTargetTopology
        | MacOsCaptureError::StaleOrForeignTarget
        | MacOsCaptureError::UnexpectedStreamStop => (ScreenSourceFailureCode::TargetLost, true),
        MacOsCaptureError::ContentUnavailable => {
            (ScreenSourceFailureCode::ContentUnavailable, false)
        }
        MacOsCaptureError::MissingImageBuffer
        | MacOsCaptureError::MissingFrameStatus
        | MacOsCaptureError::InvalidSampleBuffer
        | MacOsCaptureError::UnexpectedPixelFormat
        | MacOsCaptureError::UnexpectedFrameDimensions { .. }
        | MacOsCaptureError::InvalidRowStride
        | MacOsCaptureError::PixelBufferTooShort
        | MacOsCaptureError::PixelBufferLockFailed
        | MacOsCaptureError::InvalidTimestamp
        | MacOsCaptureError::SequenceExhausted => {
            (ScreenSourceFailureCode::InvalidNativeFrame, false)
        }
        MacOsCaptureError::NativeOperationTimedOut => {
            (ScreenSourceFailureCode::DeadlineExceeded, true)
        }
        MacOsCaptureError::DisplayCatalogUnavailable
        | MacOsCaptureError::ShareableContentUnavailable
        | MacOsCaptureError::NativeOperationCapacityUnavailable
        | MacOsCaptureError::NativeOperationWorkerUnavailable => {
            (ScreenSourceFailureCode::AdapterUnavailable, true)
        }
        _ => (ScreenSourceFailureCode::NativeOperationFailed, false),
    }
}

#[cfg(test)]
mod tests {
    use frame_media::{
        BoundScreenCaptureSource, CaptureQueueOverflow, ColorSpace, CursorPolicy,
        DisplayGeometryTransform, DpiScale, FrameMemory, LogicalRect, PhysicalRect,
        ProtectedContentPolicy, Rotation, ScreenCaptureError, ScreenCaptureQueuePolicy,
        ScreenCaptureRequest, ScreenCaptureRequestSpec, ScreenSessionId, ScreenSourceInstanceId,
        ScreenTargetBinding, ScreenTargetDescriptor, ScreenTargetEpoch, ScreenTargetId,
        ScreenTargetKind, ScreenTargetSnapshot, TargetRecoveryPolicy, VideoFrameSpec,
        negotiate_screen_capture,
    };

    use super::*;

    fn source_instance() -> ScreenSourceInstanceId {
        ScreenSourceInstanceId::new([1; 16]).expect("source identity")
    }

    fn catalog(source: ScreenSourceInstanceId) -> ScreenTargetSnapshot {
        let binding = ScreenTargetBinding::new(
            source,
            1,
            ScreenTargetEpoch::new(1).expect("target epoch"),
            ScreenTargetId::new(ScreenTargetKind::Display, [2; 16]).expect("target identity"),
        )
        .expect("target binding");
        let transform = DisplayGeometryTransform::new(
            LogicalRect::new(-100, 50, 320, 180).expect("logical bounds"),
            PhysicalRect::new(1_000, -500, 640, 360).expect("physical bounds"),
            DpiScale::new(2, 1).expect("DPI scale"),
            Rotation::Degrees0,
        )
        .expect("display transform");
        let target = ScreenTargetDescriptor::display(binding, transform).expect("display target");
        ScreenTargetSnapshot::new(source, 1, vec![target]).expect("target snapshot")
    }

    fn request(
        target: ScreenTargetDescriptor,
        protected_content: ProtectedContentPolicy,
    ) -> ScreenCaptureRequest {
        ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
            target,
            output: VideoFrameSpec {
                width: 320,
                height: 180,
                pixel_format: PixelFormat::Bgra8,
                color_space: ColorSpace::Srgb,
                nominal_frame_duration_ns: 33_333_333,
                memory: FrameMemory::Cpu,
            },
            cursor: CursorPolicy::new(CursorCaptureMode::EmbeddedInFrame, false, false)
                .expect("cursor policy"),
            excluded_windows: Vec::new(),
            queue: ScreenCaptureQueuePolicy::new(
                3,
                320 * 180 * 4 * 3,
                500_000_000,
                CaptureQueueOverflow::DropOldest,
            )
            .expect("queue policy"),
            recovery: TargetRecoveryPolicy::FailClosed,
            protected_content,
        })
        .expect("capture request")
    }

    #[test]
    fn normalized_capabilities_are_exact_and_protection_is_fail_closed() {
        let source_instance = source_instance();
        let catalog = catalog(source_instance);
        let raw = MacOsScreenCaptureSource::new(source_instance, [3; 32]).expect("raw source");
        let normalized =
            MacOsNormalizedScreenCaptureSource::new(raw, catalog.clone()).expect("normalized");
        let bound = BoundScreenCaptureSource::new(
            normalized,
            ScreenSessionId::from_csprng([4; 16]).expect("session identity"),
        )
        .expect("bound source");
        let capabilities = bound.capabilities();

        assert!(!capabilities.spec().protected_content_events);
        assert!(capabilities.spec().content_unavailable_failures);
        assert!(!capabilities.spec().cursor_image_metadata);
        assert!(!capabilities.spec().cursor_click_metadata);
        assert!(
            negotiate_screen_capture(
                capabilities,
                &catalog,
                request(
                    catalog.targets()[0].clone(),
                    ProtectedContentPolicy::FailSession,
                ),
            )
            .is_ok()
        );
        assert_eq!(
            negotiate_screen_capture(
                capabilities,
                &catalog,
                request(
                    catalog.targets()[0].clone(),
                    ProtectedContentPolicy::SuspendUntilClear,
                ),
            ),
            Err(ScreenCaptureError::ProtectedContentSignalUnavailable)
        );
    }
}
