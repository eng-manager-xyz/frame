//! Provider-neutral ownership and event adapter for Windows Graphics Capture.

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
    CALLBACK_QUEUE_CAPACITY, MAX_CAPTURE_HEIGHT, MAX_CAPTURE_WIDTH, WindowsCaptureConfig,
    WindowsCaptureError, WindowsCaptureFrame, WindowsRegionSelection, WindowsScreenCaptureSource,
};

/// WGC source bound to Frame's normalized capture-session contract.
pub struct WindowsNormalizedScreenCaptureSource {
    source: WindowsScreenCaptureSource,
    snapshot: ScreenTargetSnapshot,
    regions: Vec<WindowsRegionSelection>,
    capabilities: ScreenSourceCapabilities,
    binding: Option<ScreenSourceSessionBinding>,
    active_stream: Option<frame_media::ScreenStreamStamp>,
    stopped_tail: VecDeque<ScreenFrame<Box<[u8]>>>,
    next_control_sequence: u64,
}

impl WindowsNormalizedScreenCaptureSource {
    pub fn new(
        source: WindowsScreenCaptureSource,
        snapshot: ScreenTargetSnapshot,
    ) -> Result<Self, WindowsCaptureError> {
        if source.source_instance() != snapshot.source_instance() {
            return Err(WindowsCaptureError::StaleOrForeignTarget);
        }
        let regions = snapshot
            .targets()
            .iter()
            .filter(|target| target.kind() == ScreenTargetKind::Region)
            .map(|target| {
                WindowsRegionSelection::new(
                    target
                        .containing_display_binding()
                        .ok_or(WindowsCaptureError::InvalidRegionGeometry)?,
                    target.logical_bounds(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capabilities = capabilities(source.source_instance(), snapshot.generation())?;
        let next_control_sequence = capabilities
            .control_sequence()
            .checked_add(1)
            .ok_or(WindowsCaptureError::TopologyGenerationExhausted)?;
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
    pub const fn raw_source(&self) -> &WindowsScreenCaptureSource {
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

impl ScreenCaptureSource for WindowsNormalizedScreenCaptureSource {
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
        let permission = self.source.preflight_permission().map_err(source_failure)?;
        self.permission_observation(permission)
    }

    fn request_permission(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure> {
        self.validate_call(ticket, budget)?;
        let permission = self.source.request_permission().map_err(source_failure)?;
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
        let config = WindowsCaptureConfig::new(
            request.target().binding(),
            request.output(),
            request.cursor().mode(),
        )
        .map_err(|error| {
            let (code, retryable) = source_failure_code(&error);
            ticket.failure(code, retryable)
        })?;
        self.source
            .start(config, budget.remaining())
            .map_err(|error| {
                let (code, retryable) = source_failure_code(&error);
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
        let frames = self
            .source
            .stop_and_drain_frames(budget.remaining())
            .map_err(|error| {
                let (code, retryable) = source_failure_code(&error);
                ticket.failure(code, retryable)
            })?;
        self.active_stream = None;
        if frames.len() > CALLBACK_QUEUE_CAPACITY {
            return Err(ticket.failure(ScreenSourceFailureCode::NativeOperationFailed, false));
        }
        self.stopped_tail = frames
            .into_iter()
            .map(|frame| normalized_frame(stream, frame))
            .collect::<Result<VecDeque<_>, _>>()
            .map_err(|error| {
                let (code, retryable) = source_failure_code(&error);
                ticket.failure(code, retryable)
            })?;
        Ok(())
    }
}

fn capabilities(
    source_instance: ScreenSourceInstanceId,
    topology_generation: u64,
) -> Result<ScreenSourceCapabilities, WindowsCaptureError> {
    ScreenSourceCapabilities::new(ScreenSourceCapabilitySpec {
        contract_version: SCREEN_CAPTURE_CONTRACT_VERSION,
        source: PlatformScreenSource::WindowsGraphicsCapture,
        source_instance,
        topology_generation,
        control_epoch: ScreenControlEpoch::new(1)
            .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?,
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
        // WGC redacts protected surfaces but exposes no exact transition
        // signal. FailSession therefore remains unavailable; callers must
        // explicitly require the platform-redaction contract.
        content_unavailable_failures: false,
        platform_protected_content_redaction: true,
        window_exclusion: false,
        max_excluded_windows: 0,
        bounded_appsrc_ingress: true,
    })
    .map_err(|_| WindowsCaptureError::MediaCatalogRejected)
}

fn normalized_frame(
    stream: frame_media::ScreenStreamStamp,
    frame: WindowsCaptureFrame,
) -> Result<ScreenFrame<Box<[u8]>>, WindowsCaptureError> {
    let sequence = frame.sequence();
    let timestamp = frame.timestamp();
    let spec = frame.spec();
    let payload = frame.into_pixels().into_boxed_slice();
    let retained_bytes = u64::try_from(payload.len())
        .map_err(|_| WindowsCaptureError::FrameAllocationExceedsLimit)?;
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
    .map_err(|_| WindowsCaptureError::InvalidNativeFrame)
}

fn source_failure(error: WindowsCaptureError) -> ScreenSourceFailure {
    let (code, retryable) = source_failure_code(&error);
    ScreenSourceFailure::new(code, retryable)
}

const fn source_failure_code(error: &WindowsCaptureError) -> (ScreenSourceFailureCode, bool) {
    match error {
        WindowsCaptureError::AdapterUnavailable => {
            (ScreenSourceFailureCode::AdapterUnavailable, true)
        }
        WindowsCaptureError::TargetNoLongerAvailable
        | WindowsCaptureError::StaleTargetTopology
        | WindowsCaptureError::StaleOrForeignTarget
        | WindowsCaptureError::UnexpectedStreamStop => (ScreenSourceFailureCode::TargetLost, true),
        WindowsCaptureError::InvalidNativeFrame
        | WindowsCaptureError::InvalidRowStride
        | WindowsCaptureError::NativeBufferTooShort
        | WindowsCaptureError::InvalidTimestamp
        | WindowsCaptureError::SequenceExhausted => {
            (ScreenSourceFailureCode::InvalidNativeFrame, false)
        }
        WindowsCaptureError::CaptureStartTimedOut | WindowsCaptureError::CaptureStopTimedOut => {
            (ScreenSourceFailureCode::DeadlineExceeded, true)
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

    #[test]
    fn wgc_requires_explicit_platform_redaction_without_inventing_an_event() {
        let source_instance = ScreenSourceInstanceId::new([1; 16]).expect("source");
        let binding = ScreenTargetBinding::new(
            source_instance,
            1,
            ScreenTargetEpoch::new(1).expect("epoch"),
            ScreenTargetId::new(ScreenTargetKind::Display, [2; 16]).expect("target"),
        )
        .expect("binding");
        let target = ScreenTargetDescriptor::display(
            binding,
            DisplayGeometryTransform::new(
                LogicalRect::new(0, 0, 320, 180).expect("logical"),
                PhysicalRect::new(0, 0, 320, 180).expect("physical"),
                DpiScale::new(1, 1).expect("scale"),
                Rotation::Degrees0,
            )
            .expect("transform"),
        )
        .expect("target");
        let catalog =
            ScreenTargetSnapshot::new(source_instance, 1, vec![target.clone()]).expect("catalog");
        let raw = WindowsScreenCaptureSource::new(source_instance, [3; 32]).expect("raw");
        let normalized =
            WindowsNormalizedScreenCaptureSource::new(raw, catalog.clone()).expect("normalized");
        let bound = BoundScreenCaptureSource::new(
            normalized,
            ScreenSessionId::from_csprng([4; 16]).expect("session"),
        )
        .expect("bound");
        let request = ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
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
                .expect("cursor"),
            excluded_windows: Vec::new(),
            queue: ScreenCaptureQueuePolicy::new(
                3,
                320 * 180 * 4 * 3,
                500_000_000,
                CaptureQueueOverflow::DropOldest,
            )
            .expect("queue"),
            recovery: TargetRecoveryPolicy::FailClosed,
            protected_content: ProtectedContentPolicy::FailSession,
        })
        .expect("request");
        assert_eq!(
            negotiate_screen_capture(bound.capabilities(), &catalog, request),
            Err(ScreenCaptureError::ProtectedContentSignalUnavailable)
        );

        let target = catalog.targets()[0].clone();
        let request = ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
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
                .expect("cursor"),
            excluded_windows: Vec::new(),
            queue: ScreenCaptureQueuePolicy::new(
                3,
                320 * 180 * 4 * 3,
                500_000_000,
                CaptureQueueOverflow::DropOldest,
            )
            .expect("queue"),
            recovery: TargetRecoveryPolicy::FailClosed,
            protected_content: ProtectedContentPolicy::RequirePlatformRedaction,
        })
        .expect("request");
        assert!(negotiate_screen_capture(bound.capabilities(), &catalog, request).is_ok());
    }
}
