use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use frame_media::{
    DisplayGeometryTransform, DpiScale, LogicalRect, PermissionPreflight, PhysicalRect, Rotation,
    ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetSnapshot,
};
use frame_windows_capture_ffi::{
    capture_item_for_monitor, capture_item_for_window, enumerate_displays,
    enumerate_non_frame_windows, request_worker_stop,
};
use wgc::{Frame, FrameSize, PixelFormat as WgcPixelFormat, Wgc, WgcSettings};
use zeroize::Zeroizing;

use crate::{
    CALLBACK_QUEUE_CAPACITY, MAX_CAPTURE_HEIGHT, MAX_CAPTURE_WIDTH, TimestampNormalizer,
    WindowsCaptureConfig, WindowsCaptureDiagnostics, WindowsCaptureError, WindowsCaptureFrame,
    WindowsRegionSelection,
    target_catalog::{
        NativeDisplayRecord, NativeTargetRecord, NativeWindowRecord, assemble_records,
        build_catalog, resolve_region_selections,
    },
};

const TERMINAL_NONE: u8 = 0;
const TERMINAL_TARGET_LOST: u8 = 1;
const TERMINAL_NATIVE_FAILURE: u8 = 2;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Default)]
struct DiagnosticCounters {
    dropped_callback_frames: AtomicU64,
    callback_frames_after_stop: AtomicU64,
    invalid_native_frames: AtomicU64,
    timestamp_discontinuities: AtomicU64,
    target_closed_events: AtomicU64,
    unexpected_native_stops: AtomicU64,
    start_timeouts: AtomicU64,
    stop_timeouts: AtomicU64,
}

impl DiagnosticCounters {
    fn snapshot(&self) -> WindowsCaptureDiagnostics {
        WindowsCaptureDiagnostics {
            dropped_callback_frames: self.dropped_callback_frames.load(Ordering::Relaxed),
            callback_frames_after_stop: self.callback_frames_after_stop.load(Ordering::Relaxed),
            invalid_native_frames: self.invalid_native_frames.load(Ordering::Relaxed),
            timestamp_discontinuities: self.timestamp_discontinuities.load(Ordering::Relaxed),
            target_closed_events: self.target_closed_events.load(Ordering::Relaxed),
            unexpected_native_stops: self.unexpected_native_stops.load(Ordering::Relaxed),
            start_timeouts: self.start_timeouts.load(Ordering::Relaxed),
            stop_timeouts: self.stop_timeouts.load(Ordering::Relaxed),
        }
    }
}

struct ActiveCapture {
    worker: JoinHandle<()>,
    receiver: Receiver<WindowsCaptureFrame>,
    stop_requested: Arc<AtomicBool>,
    terminal: Arc<AtomicU8>,
}

enum CaptureLifecycle {
    Ready,
    Running(ActiveCapture),
    TeardownUnconfirmed(Option<JoinHandle<()>>),
}

struct StopFailure {
    error: WindowsCaptureError,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy)]
enum FrameCrop {
    Full {
        width: u32,
        height: u32,
    },
    Region {
        input_width: u32,
        input_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl FrameCrop {
    const fn input_dimensions(self) -> (u32, u32) {
        match self {
            Self::Full { width, height } => (width, height),
            Self::Region {
                input_width,
                input_height,
                ..
            } => (input_width, input_height),
        }
    }

    const fn output_dimensions(self) -> (u32, u32) {
        match self {
            Self::Full { width, height } => (width, height),
            Self::Region { width, height, .. } => (width, height),
        }
    }
}

/// Safe display, window, and display-relative-region Windows Graphics Capture
/// source. Construct one instance per normalized session.
pub struct WindowsScreenCaptureSource {
    source_instance: ScreenSourceInstanceId,
    session_secret: Zeroizing<[u8; 32]>,
    topology_generation: u64,
    catalog_records: Option<Vec<NativeTargetRecord>>,
    target_map: BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
    diagnostics: Arc<DiagnosticCounters>,
    capture: CaptureLifecycle,
}

impl WindowsScreenCaptureSource {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        session_secret: [u8; 32],
    ) -> Result<Self, WindowsCaptureError> {
        if session_secret.iter().all(|byte| *byte == 0) {
            return Err(WindowsCaptureError::InvalidSessionSecret);
        }
        Ok(Self {
            source_instance,
            session_secret: Zeroizing::new(session_secret),
            topology_generation: 0,
            catalog_records: None,
            target_map: BTreeMap::new(),
            diagnostics: Arc::new(DiagnosticCounters::default()),
            capture: CaptureLifecycle::Ready,
        })
    }

    #[must_use]
    pub const fn source_instance(&self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    pub fn preflight_permission(&mut self) -> Result<PermissionPreflight, WindowsCaptureError> {
        if wgc::is_wgc_supported().map_err(|_| WindowsCaptureError::AdapterUnavailable)? {
            Ok(PermissionPreflight::Granted)
        } else {
            Ok(PermissionPreflight::Restricted)
        }
    }

    pub fn request_permission(&mut self) -> Result<PermissionPreflight, WindowsCaptureError> {
        // Programmatic WGC monitor/window capture has no Frame-owned permission
        // prompt. This revalidates availability without opening a picker.
        self.preflight_permission()
    }

    pub fn enumerate_targets(
        &mut self,
        regions: &[WindowsRegionSelection],
    ) -> Result<ScreenTargetSnapshot, WindowsCaptureError> {
        self.ensure_catalog_mutable()?;
        let regions = resolve_region_selections(&self.target_map, regions)?;
        let displays = active_display_records()?;
        let windows = enumerate_non_frame_windows()
            .map_err(|_| WindowsCaptureError::AdapterUnavailable)?
            .into_iter()
            .map(|window| {
                let bounds =
                    LogicalRect::new(window.x(), window.y(), window.width(), window.height())
                        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
                Ok(NativeWindowRecord::new(window.native_id(), bounds))
            })
            .collect::<Result<Vec<_>, WindowsCaptureError>>()?;
        let records = assemble_records(displays, windows, &regions)?;
        self.install_catalog(records)
    }

    pub fn start(
        &mut self,
        config: WindowsCaptureConfig,
        timeout: Duration,
    ) -> Result<(), WindowsCaptureError> {
        self.ensure_ready()?;
        if config.target().source_instance() != self.source_instance {
            return Err(WindowsCaptureError::StaleOrForeignTarget);
        }
        let target = self
            .target_map
            .get(&config.target())
            .copied()
            .ok_or(WindowsCaptureError::StaleOrForeignTarget)?;
        target.validate_output(config.output().width, config.output().height)?;
        if self.preflight_permission()? != PermissionPreflight::Granted {
            return Err(WindowsCaptureError::AdapterUnavailable);
        }
        if !wgc::is_cursor_configurable().map_err(|_| WindowsCaptureError::AdapterUnavailable)? {
            return Err(WindowsCaptureError::AdapterUnavailable);
        }

        let (item, crop) = capture_item_and_crop(target)?;
        let settings = WgcSettings {
            pixel_format: WgcPixelFormat::BGRA8,
            frame_queue_length: i32::try_from(CALLBACK_QUEUE_CAPACITY)
                .map_err(|_| WindowsCaptureError::CaptureStartFailed)?,
            capture_cursor: Some(matches!(
                config.cursor(),
                frame_media::CursorCaptureMode::EmbeddedInFrame
            )),
            display_border: wgc::is_border_configurable()
                .map_err(|_| WindowsCaptureError::AdapterUnavailable)?
                .then_some(false),
            include_secondary_windows: wgc::is_include_secondary_windows_configurable()
                .map_err(|_| WindowsCaptureError::AdapterUnavailable)?
                .then_some(false),
            min_update_interval: wgc::is_min_update_interval_configurable()
                .map_err(|_| WindowsCaptureError::AdapterUnavailable)?
                .then_some(Duration::from_nanos(
                    config.output().nominal_frame_duration_ns,
                )),
            ..WgcSettings::default()
        };
        let (sender, receiver) = sync_channel(CALLBACK_QUEUE_CAPACITY);
        let (started_sender, started_receiver) = sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let terminal = Arc::new(AtomicU8::new(TERMINAL_NONE));
        let diagnostics = Arc::clone(&self.diagnostics);
        let worker_stop = Arc::clone(&stop_requested);
        let worker_terminal = Arc::clone(&terminal);
        let output = config.output();
        let target_binding = config.target();
        let clock_epoch = Instant::now();
        let worker = thread::Builder::new()
            .name("frame-wgc-capture".to_owned())
            .spawn(move || {
                run_capture_worker(
                    item,
                    settings,
                    crop,
                    target_binding,
                    output,
                    clock_epoch,
                    sender,
                    started_sender,
                    worker_stop,
                    worker_terminal,
                    diagnostics,
                );
            })
            .map_err(|_| WindowsCaptureError::CaptureStartFailed)?;

        match started_receiver.recv_timeout(timeout) {
            Ok(Ok(())) => {
                self.capture = CaptureLifecycle::Running(ActiveCapture {
                    worker,
                    receiver,
                    stop_requested,
                    terminal,
                });
                Ok(())
            }
            Ok(Err(())) | Err(RecvTimeoutError::Disconnected) => {
                wait_for_finished_worker(worker, timeout)
                    .map_err(|_| WindowsCaptureError::CaptureStartFailed)?;
                Err(WindowsCaptureError::CaptureStartFailed)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.diagnostics
                    .start_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                stop_requested.store(true, Ordering::Release);
                let _ = request_worker_stop(&worker);
                self.capture = CaptureLifecycle::TeardownUnconfirmed(Some(worker));
                Err(WindowsCaptureError::CaptureStartTimedOut)
            }
        }
    }

    pub fn poll_frame(&mut self) -> Result<Option<WindowsCaptureFrame>, WindowsCaptureError> {
        let CaptureLifecycle::Running(active) = &mut self.capture else {
            return match self.capture {
                CaptureLifecycle::Ready => Err(WindowsCaptureError::NotRunning),
                CaptureLifecycle::TeardownUnconfirmed(_) => {
                    Err(WindowsCaptureError::CaptureTeardownUnconfirmed)
                }
                CaptureLifecycle::Running(_) => unreachable!(),
            };
        };
        match active.receiver.try_recv() {
            Ok(frame) => return Ok(Some(frame)),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) if !active.worker.is_finished() => {
                return Err(WindowsCaptureError::CallbackQueueDisconnected);
            }
            Err(TryRecvError::Disconnected) => {}
        }
        match active.terminal.swap(TERMINAL_NONE, Ordering::AcqRel) {
            TERMINAL_TARGET_LOST => {
                self.diagnostics
                    .target_closed_events
                    .fetch_add(1, Ordering::Relaxed);
                Err(WindowsCaptureError::TargetNoLongerAvailable)
            }
            TERMINAL_NATIVE_FAILURE => Err(WindowsCaptureError::InvalidNativeFrame),
            TERMINAL_NONE if active.worker.is_finished() => {
                self.diagnostics
                    .unexpected_native_stops
                    .fetch_add(1, Ordering::Relaxed);
                Err(WindowsCaptureError::UnexpectedStreamStop)
            }
            TERMINAL_NONE => Ok(None),
            _ => Err(WindowsCaptureError::UnexpectedStreamStop),
        }
    }

    pub fn stop_and_drain_frames(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<WindowsCaptureFrame>, WindowsCaptureError> {
        let lifecycle = std::mem::replace(&mut self.capture, CaptureLifecycle::Ready);
        match lifecycle {
            CaptureLifecycle::Ready => Ok(Vec::new()),
            CaptureLifecycle::Running(active) => match stop_active_capture(active, timeout) {
                Ok(frames) => Ok(frames),
                Err(failure) => {
                    if failure.error == WindowsCaptureError::CaptureStopTimedOut {
                        self.diagnostics
                            .stop_timeouts
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(worker) = failure.worker {
                        self.capture = CaptureLifecycle::TeardownUnconfirmed(Some(worker));
                    }
                    Err(failure.error)
                }
            },
            CaptureLifecycle::TeardownUnconfirmed(worker) => {
                let Some(worker) = worker else {
                    return Err(WindowsCaptureError::CaptureTeardownUnconfirmed);
                };
                match stop_worker(worker, timeout) {
                    Ok(()) => Ok(Vec::new()),
                    Err(failure) => {
                        if let Some(worker) = failure.worker {
                            self.capture = CaptureLifecycle::TeardownUnconfirmed(Some(worker));
                        }
                        Err(failure.error)
                    }
                }
            }
        }
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self.capture, CaptureLifecycle::Running(_))
    }

    #[must_use]
    pub fn diagnostics(&self) -> WindowsCaptureDiagnostics {
        self.diagnostics.snapshot()
    }

    fn ensure_ready(&self) -> Result<(), WindowsCaptureError> {
        match self.capture {
            CaptureLifecycle::Ready => Ok(()),
            CaptureLifecycle::Running(_) => Err(WindowsCaptureError::AlreadyRunning),
            CaptureLifecycle::TeardownUnconfirmed(_) => {
                Err(WindowsCaptureError::CaptureTeardownUnconfirmed)
            }
        }
    }

    fn ensure_catalog_mutable(&self) -> Result<(), WindowsCaptureError> {
        self.ensure_ready()
    }

    fn install_catalog(
        &mut self,
        records: Vec<NativeTargetRecord>,
    ) -> Result<ScreenTargetSnapshot, WindowsCaptureError> {
        let generation = if self.catalog_records.as_ref() == Some(&records) {
            self.topology_generation
        } else {
            self.topology_generation
                .checked_add(1)
                .ok_or(WindowsCaptureError::TopologyGenerationExhausted)?
        };
        let catalog = build_catalog(
            &self.session_secret,
            self.source_instance,
            generation,
            &records,
        )?;
        let (snapshot, target_map) = catalog.into_parts();
        self.topology_generation = generation;
        self.catalog_records = Some(records);
        self.target_map = target_map;
        Ok(snapshot)
    }
}

impl Drop for WindowsScreenCaptureSource {
    fn drop(&mut self) {
        let lifecycle = std::mem::replace(&mut self.capture, CaptureLifecycle::Ready);
        match lifecycle {
            CaptureLifecycle::Running(active) => {
                let _ = stop_active_capture(active, Duration::from_millis(250));
            }
            CaptureLifecycle::TeardownUnconfirmed(Some(worker)) => {
                let _ = stop_worker(worker, Duration::from_millis(250));
            }
            CaptureLifecycle::Ready | CaptureLifecycle::TeardownUnconfirmed(None) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_capture_worker(
    item: windows::Graphics::Capture::GraphicsCaptureItem,
    settings: WgcSettings,
    crop: FrameCrop,
    target: ScreenTargetBinding,
    output: frame_media::VideoFrameSpec,
    clock_epoch: Instant,
    sender: SyncSender<WindowsCaptureFrame>,
    started: SyncSender<Result<(), ()>>,
    stop_requested: Arc<AtomicBool>,
    terminal: Arc<AtomicU8>,
    diagnostics: Arc<DiagnosticCounters>,
) {
    let mut capture = match Wgc::new(item, settings) {
        Ok(capture) => capture,
        Err(_) => {
            let _ = started.send(Err(()));
            return;
        }
    };
    if started.send(Ok(())).is_err() {
        return;
    }
    let mut sequence = 0_u64;
    let mut timestamps = TimestampNormalizer::default();
    while let Some(result) = capture.next() {
        if stop_requested.load(Ordering::Acquire) {
            break;
        }
        let frame = match result.and_then(|frame| {
            build_owned_frame(
                &frame,
                crop,
                target,
                output,
                clock_epoch,
                &mut sequence,
                &mut timestamps,
            )
            .map_err(|_| wgc::WgcError::NoItemSelected)
        }) {
            Ok(frame) => frame,
            Err(_) => {
                diagnostics
                    .invalid_native_frames
                    .fetch_add(1, Ordering::Relaxed);
                terminal.store(TERMINAL_NATIVE_FAILURE, Ordering::Release);
                return;
            }
        };
        if frame.timestamp().discontinuity {
            diagnostics
                .timestamp_discontinuities
                .fetch_add(1, Ordering::Relaxed);
        }
        match sender.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                diagnostics
                    .dropped_callback_frames
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                diagnostics
                    .callback_frames_after_stop
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }
    if !stop_requested.load(Ordering::Acquire) {
        terminal.store(TERMINAL_TARGET_LOST, Ordering::Release);
    }
}

fn build_owned_frame(
    frame: &Frame,
    crop: FrameCrop,
    target: ScreenTargetBinding,
    output: frame_media::VideoFrameSpec,
    clock_epoch: Instant,
    sequence: &mut u64,
    timestamps: &mut TimestampNormalizer,
) -> Result<WindowsCaptureFrame, WindowsCaptureError> {
    let FrameSize { width, height } = frame
        .size()
        .map_err(|_| WindowsCaptureError::InvalidNativeFrame)?;
    if (width, height) != crop.input_dimensions()
        || width == 0
        || height == 0
        || width > MAX_CAPTURE_WIDTH
        || height > MAX_CAPTURE_HEIGHT
    {
        return Err(WindowsCaptureError::InvalidNativeFrame);
    }
    let render_time = frame
        .render_time()
        .map_err(|_| WindowsCaptureError::InvalidTimestamp)?;
    let source_pts_ns = render_time
        .checked_duration_since(clock_epoch)
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok());
    let raw_ns = source_pts_ns.ok_or(WindowsCaptureError::InvalidTimestamp)?;
    let timestamp = timestamps.normalize(raw_ns, output.nominal_frame_duration_ns)?;
    let pixels = frame
        .read_pixels(None)
        .map_err(|_| WindowsCaptureError::InvalidNativeFrame)?;
    let pixels = match crop {
        FrameCrop::Full { .. } => pixels,
        FrameCrop::Region {
            input_width,
            input_height,
            x,
            y,
            width,
            height,
        } => crop_tight_bgra(&pixels, input_width, input_height, x, y, width, height)?,
    };
    if crop.output_dimensions() != (output.width, output.height)
        || pixels.len() != super::bgra_frame_bytes(output.width, output.height)?
    {
        return Err(WindowsCaptureError::InvalidNativeFrame);
    }
    *sequence = sequence
        .checked_add(1)
        .ok_or(WindowsCaptureError::SequenceExhausted)?;
    Ok(WindowsCaptureFrame {
        target,
        sequence: *sequence,
        source_pts_ns: Some(raw_ns),
        timestamp,
        spec: output,
        pixels,
    })
}

fn crop_tight_bgra(
    pixels: &[u8],
    input_width: u32,
    input_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, WindowsCaptureError> {
    let input_row = usize::try_from(input_width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    let expected_input = input_row
        .checked_mul(
            usize::try_from(input_height)
                .map_err(|_| WindowsCaptureError::FrameAllocationExceedsLimit)?,
        )
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    if pixels.len() != expected_input
        || x.checked_add(width).is_none_or(|right| right > input_width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > input_height)
    {
        return Err(WindowsCaptureError::InvalidNativeFrame);
    }
    let output_row = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    let output_len = output_row
        .checked_mul(
            usize::try_from(height)
                .map_err(|_| WindowsCaptureError::FrameAllocationExceedsLimit)?,
        )
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    let mut output = Vec::with_capacity(output_len);
    for row in y..y + height {
        let start = usize::try_from(row)
            .ok()
            .and_then(|row| row.checked_mul(input_row))
            .and_then(|offset| {
                usize::try_from(x)
                    .ok()
                    .and_then(|x| x.checked_mul(4))
                    .and_then(|x| offset.checked_add(x))
            })
            .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
        output.extend_from_slice(&pixels[start..start + output_row]);
    }
    Ok(output)
}

fn capture_item_and_crop(
    target: NativeTargetRecord,
) -> Result<(windows::Graphics::Capture::GraphicsCaptureItem, FrameCrop), WindowsCaptureError> {
    match target {
        NativeTargetRecord::Display(display) => {
            let bounds = display.transform().physical_bounds();
            let item = capture_item_for_monitor(display.native_id())
                .map_err(|_| WindowsCaptureError::TargetNoLongerAvailable)?;
            Ok((
                item,
                FrameCrop::Full {
                    width: bounds.width(),
                    height: bounds.height(),
                },
            ))
        }
        NativeTargetRecord::Window(window) => {
            let bounds = window.logical_bounds();
            let item = capture_item_for_window(window.native_id())
                .map_err(|_| WindowsCaptureError::TargetNoLongerAvailable)?;
            Ok((
                item,
                FrameCrop::Full {
                    width: bounds.width(),
                    height: bounds.height(),
                },
            ))
        }
        NativeTargetRecord::Region {
            display,
            logical_bounds,
        } => {
            let display_bounds = display.transform().physical_bounds();
            let region = display
                .transform()
                .logical_rect_to_physical(logical_bounds)
                .map_err(|_| WindowsCaptureError::InvalidRegionGeometry)?;
            let x = u32::try_from(region.x())
                .map_err(|_| WindowsCaptureError::InvalidRegionGeometry)?;
            let y = u32::try_from(region.y())
                .map_err(|_| WindowsCaptureError::InvalidRegionGeometry)?;
            let item = capture_item_for_monitor(display.native_id())
                .map_err(|_| WindowsCaptureError::TargetNoLongerAvailable)?;
            Ok((
                item,
                FrameCrop::Region {
                    input_width: display_bounds.width(),
                    input_height: display_bounds.height(),
                    x,
                    y,
                    width: region.width(),
                    height: region.height(),
                },
            ))
        }
    }
}

fn stop_active_capture(
    active: ActiveCapture,
    timeout: Duration,
) -> Result<Vec<WindowsCaptureFrame>, StopFailure> {
    active.stop_requested.store(true, Ordering::Release);
    let ActiveCapture {
        worker,
        receiver,
        stop_requested: _,
        terminal: _,
    } = active;
    stop_worker(worker, timeout)?;
    let mut frames = Vec::with_capacity(CALLBACK_QUEUE_CAPACITY);
    loop {
        match receiver.try_recv() {
            Ok(frame) if frames.len() < CALLBACK_QUEUE_CAPACITY => frames.push(frame),
            Ok(_) => {
                return Err(StopFailure {
                    error: WindowsCaptureError::CaptureStopFailed,
                    worker: None,
                });
            }
            Err(TryRecvError::Empty) => {
                return Err(StopFailure {
                    error: WindowsCaptureError::CallbackQueueDisconnected,
                    worker: None,
                });
            }
            Err(TryRecvError::Disconnected) => return Ok(frames),
        }
    }
}

fn stop_worker(worker: JoinHandle<()>, timeout: Duration) -> Result<(), StopFailure> {
    if !worker.is_finished() && request_worker_stop(&worker).is_err() && !worker.is_finished() {
        return Err(StopFailure {
            error: WindowsCaptureError::CaptureStopFailed,
            worker: Some(worker),
        });
    }
    let deadline = Instant::now().checked_add(timeout);
    while !worker.is_finished() {
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(StopFailure {
                error: WindowsCaptureError::CaptureStopTimedOut,
                worker: Some(worker),
            });
        }
        thread::park_timeout(WORKER_POLL_INTERVAL);
    }
    worker.join().map_err(|_| StopFailure {
        error: WindowsCaptureError::CaptureStopFailed,
        worker: None,
    })
}

fn wait_for_finished_worker(
    worker: JoinHandle<()>,
    timeout: Duration,
) -> Result<(), WindowsCaptureError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(WindowsCaptureError::CaptureStartTimedOut)?;
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            return Err(WindowsCaptureError::CaptureStartTimedOut);
        }
        thread::park_timeout(WORKER_POLL_INTERVAL);
    }
    worker
        .join()
        .map_err(|_| WindowsCaptureError::CaptureStartFailed)
}

fn active_display_records() -> Result<Vec<NativeDisplayRecord>, WindowsCaptureError> {
    enumerate_displays()
        .map_err(|_| WindowsCaptureError::AdapterUnavailable)?
        .into_iter()
        .take(frame_media::MAX_SCREEN_TARGETS.saturating_add(1))
        .map(|display| {
            Ok(NativeDisplayRecord::new(
                display.native_id(),
                display_transform(display)?,
            ))
        })
        .collect()
}

fn display_transform(
    display: frame_windows_capture_ffi::NativeDisplay,
) -> Result<DisplayGeometryTransform, WindowsCaptureError> {
    let rotation = rotation_from_degrees(display.rotation_degrees())?;
    let scale = f64::from(display.scale_numerator()) / f64::from(display.scale_denominator());
    let (unrotated_width, unrotated_height) = match rotation {
        Rotation::Degrees0 | Rotation::Degrees180 => (display.width(), display.height()),
        Rotation::Degrees90 | Rotation::Degrees270 => (display.height(), display.width()),
    };
    let logical_width = scaled_coordinate(unrotated_width, scale)?;
    let logical_height = scaled_coordinate(unrotated_height, scale)?;
    let logical_x = scaled_origin(display.x(), scale)?;
    let logical_y = scaled_origin(display.y(), scale)?;
    let physical = PhysicalRect::new(0, 0, display.width(), display.height())
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
    let logical = LogicalRect::new(logical_x, logical_y, logical_width, logical_height)
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
    let rational = DpiScale::new(unrotated_width, logical_width)
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
    DisplayGeometryTransform::new(logical, physical, rational, rotation).or_else(|_| {
        // Some Windows modes round the two logical axes differently. Preserve
        // exact pixel geometry and rotation instead of advertising a false DPI
        // ratio; hardware evidence can then identify the downgraded descriptor.
        let fallback =
            LogicalRect::new(display.x(), display.y(), unrotated_width, unrotated_height)
                .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
        DisplayGeometryTransform::new(
            fallback,
            physical,
            DpiScale::new(1, 1).map_err(|_| WindowsCaptureError::MediaCatalogRejected)?,
            rotation,
        )
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_coordinate(value: u32, scale: f64) -> Result<u32, WindowsCaptureError> {
    let logical = (f64::from(value) / scale).round();
    if logical < 1.0 || logical > f64::from(u32::MAX) {
        return Err(WindowsCaptureError::MediaCatalogRejected);
    }
    Ok(logical as u32)
}

#[allow(clippy::cast_possible_truncation)]
fn scaled_origin(value: i32, scale: f64) -> Result<i32, WindowsCaptureError> {
    let logical = (f64::from(value) / scale).round();
    if logical < f64::from(i32::MIN) || logical > f64::from(i32::MAX) {
        return Err(WindowsCaptureError::MediaCatalogRejected);
    }
    Ok(logical as i32)
}

fn rotation_from_degrees(value: u16) -> Result<Rotation, WindowsCaptureError> {
    match value {
        0 => Ok(Rotation::Degrees0),
        90 => Ok(Rotation::Degrees90),
        180 => Ok(Rotation::Degrees180),
        270 => Ok(Rotation::Degrees270),
        _ => Err(WindowsCaptureError::MediaCatalogRejected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_rejects_overcapture_and_copies_exact_rows() {
        let pixels: Vec<u8> = (0..48).collect();
        assert_eq!(
            crop_tight_bgra(&pixels, 4, 3, 1, 1, 2, 2).expect("crop"),
            vec![
                20, 21, 22, 23, 24, 25, 26, 27, 36, 37, 38, 39, 40, 41, 42, 43
            ]
        );
        assert_eq!(
            crop_tight_bgra(&pixels, 4, 3, 3, 0, 2, 1),
            Err(WindowsCaptureError::InvalidNativeFrame)
        );
    }
}
