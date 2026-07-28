//! Fair, bounded ownership loop for ScreenCaptureKit video plus system audio.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, Ordering},
        mpsc::Receiver,
    },
    thread,
    time::{Duration, Instant},
};

use frame_macos_av_capture::{
    MacOsOptionalInputRecording, MacOsSystemAudioChunk, MacOsSystemAudioDiagnostics,
    MacOsSystemAudioSource, MacOsSystemAudioStopError,
};
use frame_macos_screen_capture::{
    MacOsCaptureDiagnostics, MacOsCaptureFrame, MacOsCaptureStopError, MacOsScreenCaptureSource,
};
use frame_media::{
    AudioSourceMixSettings, AvDiagnostic, AvSourceClass, AvSyncPolicy, BgraScreenFrame,
    CalibrationSample, CancellationToken, F32StereoAudioChunk, FrameTimestamp, LatencyConfidence,
    MonotonicTimeNs, NativeAvGraphOutputKind, NativeAvGraphOutputSample, NativeAvGraphTeardown,
    NativeAvRuntimeOutcome, NativeAvSourceTeardown, ScreenAudioRecording, ScreenRecording,
    ScreenRecordingError, SourceLatency, SourceTimebase, StartupCalibration,
};

use super::{
    CompletedRecordingArtifact, DesktopStudioRecording, NativeDesktopBackendError,
    PendingRecordingGraph, WORKER_IDLE_POLL, WorkerCompletion, WorkerControl, WorkerOutcome,
    all_av_teardown_confirmed, diagnostic_delta, diagnostics_failed, map_capture_error,
    map_recording_error, map_system_audio_error, recording_finish_teardown_confirmed,
    system_audio_diagnostic_delta, system_audio_diagnostics_failed,
};
use crate::DeviceClass;

const STARTUP_CALIBRATION_TIMEOUT: Duration = Duration::from_millis(80);
const SCREEN_STARTUP_CALIBRATION_SAMPLES: usize = 5;
const OPTIONAL_STOP_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct AvWorkerTelemetry {
    pub(super) screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    pub(super) audio_diagnostic_baseline: MacOsSystemAudioDiagnostics,
    pub(super) system_audio_meter: Arc<AtomicU16>,
}

pub(super) struct OptionalWorkerTelemetry {
    pub(super) screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    pub(super) microphone_meter: Arc<AtomicU16>,
    pub(super) system_audio_meter: Arc<AtomicU16>,
    pub(super) camera_active: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct SharedScreenClock {
    source_origin_ns: Option<u64>,
    calibration: Vec<CalibrationSample>,
    timebase: Option<SourceTimebase>,
}

impl SharedScreenClock {
    fn normalize(
        &mut self,
        frame: MacOsCaptureFrame,
    ) -> Result<Option<(u64, FrameTimestamp, Vec<u8>)>, NativeDesktopBackendError> {
        let sequence = frame.sequence();
        let source_pts_ns = frame
            .source_pts_ns()
            .ok_or(NativeDesktopBackendError::Internal)?;
        let master_arrival = MonotonicTimeNs::new(
            frame
                .master_arrival_ns()
                .ok_or(NativeDesktopBackendError::Internal)?,
        );
        let duration_ns = frame.timestamp().duration_ns;
        let discontinuity = frame.timestamp().discontinuity;
        let origin = *self.source_origin_ns.get_or_insert(source_pts_ns);
        let source_pts_ns = source_pts_ns
            .checked_sub(origin)
            .ok_or(NativeDesktopBackendError::Internal)?;
        let latency = SourceLatency {
            reported_ns: 0,
            confidence: LatencyConfidence::Unknown,
        };
        if self.timebase.is_none() {
            self.calibration.push(CalibrationSample {
                master_arrival,
                source_pts_ns,
                latency,
            });
            if self.calibration.len() < SCREEN_STARTUP_CALIBRATION_SAMPLES {
                return Ok(None);
            }
            if self.calibration.len() > SCREEN_STARTUP_CALIBRATION_SAMPLES {
                return Err(NativeDesktopBackendError::Internal);
            }
            let startup = StartupCalibration::measure(&self.calibration)
                .map_err(|_| NativeDesktopBackendError::Internal)?;
            let anchor = *self
                .calibration
                .last()
                .ok_or(NativeDesktopBackendError::Internal)?;
            self.timebase = Some(
                SourceTimebase::new(AvSyncPolicy::default(), startup, anchor)
                    .map_err(|_| NativeDesktopBackendError::Internal)?,
            );
            self.calibration.clear();
            return Ok(None);
        }
        let timestamp = self
            .timebase
            .as_mut()
            .ok_or(NativeDesktopBackendError::Internal)?
            .observe(
                source_pts_ns,
                duration_ns,
                master_arrival,
                latency,
                discontinuity,
            )
            .map_err(|_| NativeDesktopBackendError::Internal)?
            .frame;
        Ok(Some((sequence, timestamp, frame.into_pixels())))
    }

    fn pause(&mut self, now: MonotonicTimeNs) -> Result<(), NativeDesktopBackendError> {
        if let Some(timebase) = self.timebase.as_mut() {
            timebase
                .pause(now)
                .map_err(|_| NativeDesktopBackendError::Internal)?;
        } else {
            self.source_origin_ns = None;
            self.calibration.clear();
        }
        Ok(())
    }

    fn resume(&mut self, now: MonotonicTimeNs) -> Result<(), NativeDesktopBackendError> {
        if let Some(timebase) = self.timebase.as_mut() {
            timebase
                .resume(now)
                .map_err(|_| NativeDesktopBackendError::Internal)?;
        } else {
            self.source_origin_ns = None;
            self.calibration.clear();
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct SharedClockNormalizer {
    source_origin_ns: u64,
    last_video_end_ns: u64,
    last_audio_end_ns: u64,
}

impl SharedClockNormalizer {
    const fn new(screen_source_pts_ns: u64, audio_source_pts_ns: u64) -> Self {
        Self {
            source_origin_ns: if screen_source_pts_ns < audio_source_pts_ns {
                screen_source_pts_ns
            } else {
                audio_source_pts_ns
            },
            last_video_end_ns: 0,
            last_audio_end_ns: 0,
        }
    }

    const fn screen_only(screen_source_pts_ns: u64) -> Self {
        Self {
            source_origin_ns: screen_source_pts_ns,
            last_video_end_ns: 0,
            last_audio_end_ns: 0,
        }
    }

    fn normalize_video(
        &mut self,
        source_pts_ns: u64,
        source_timestamp: FrameTimestamp,
    ) -> Result<FrameTimestamp, ScreenRecordingError> {
        normalize_timestamp(
            self.source_origin_ns,
            &mut self.last_video_end_ns,
            source_pts_ns,
            source_timestamp.duration_ns,
            source_timestamp.discontinuity,
        )
    }

    fn normalize_audio(
        &mut self,
        chunk: MacOsSystemAudioChunk,
    ) -> Result<F32StereoAudioChunk, ScreenRecordingError> {
        let timestamp = normalize_timestamp(
            self.source_origin_ns,
            &mut self.last_audio_end_ns,
            chunk.source_pts_ns(),
            chunk.duration_ns(),
            chunk.discontinuity(),
        )?;
        F32StereoAudioChunk::new(
            chunk.sequence(),
            timestamp.pts_ns,
            timestamp.duration_ns,
            timestamp.discontinuity,
            chunk.into_samples_f32le(),
        )
    }
}

pub(super) fn run_optional_av_capture_worker(
    mut source: MacOsScreenCaptureSource,
    mut optional: MacOsOptionalInputRecording,
    mut recording: PendingRecordingGraph,
    mut studio: Option<DesktopStudioRecording>,
    control: Receiver<WorkerControl>,
    telemetry: OptionalWorkerTelemetry,
) -> WorkerCompletion {
    let mut screen_clock = SharedScreenClock::default();
    let mut paused = false;
    let mut camera_enabled = optional.selection().camera;
    loop {
        match control.try_recv() {
            Ok(WorkerControl::Stop) => {
                let outcome = finish_optional_worker(
                    &mut source,
                    &mut optional,
                    recording,
                    studio,
                    &mut screen_clock,
                    &telemetry,
                    camera_enabled,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Cancel) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let outcome = cancel_optional_worker(
                    &mut source,
                    &mut optional,
                    recording,
                    studio,
                    &telemetry,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Pause(reply)) => {
                let now = optional.master_now();
                let result = screen_clock.pause(now).and_then(|()| {
                    optional
                        .pause()
                        .map_err(|_| NativeDesktopBackendError::Internal)
                });
                let _ = reply.send(result);
                if result.is_err() {
                    let outcome = fail_optional_worker(
                        &mut source,
                        &mut optional,
                        recording,
                        studio,
                        &telemetry,
                        NativeDesktopBackendError::Internal,
                    );
                    return WorkerCompletion { outcome };
                }
                paused = true;
            }
            Ok(WorkerControl::Resume(reply)) => {
                let result = optional
                    .resume()
                    .map_err(|_| NativeDesktopBackendError::Internal)
                    .and_then(|()| screen_clock.resume(optional.master_now()));
                let _ = reply.send(result);
                if result.is_err() {
                    let outcome = fail_optional_worker(
                        &mut source,
                        &mut optional,
                        recording,
                        studio,
                        &telemetry,
                        NativeDesktopBackendError::Internal,
                    );
                    return WorkerCompletion { outcome };
                }
                paused = false;
            }
            Ok(WorkerControl::Input { request, reply }) => {
                let result = match request.class {
                    DeviceClass::Microphone if optional.selection().microphone => optional
                        .set_audio_mix(
                            AvSourceClass::Microphone,
                            AudioSourceMixSettings {
                                gain_milli: request.gain_milli,
                                muted: request.muted || !request.enabled,
                                ramp_frames: 480,
                            },
                        )
                        .map_err(|_| NativeDesktopBackendError::Internal),
                    DeviceClass::SystemAudio if optional.selection().system_audio => optional
                        .set_audio_mix(
                            AvSourceClass::SystemAudio,
                            AudioSourceMixSettings {
                                gain_milli: request.gain_milli,
                                muted: request.muted || !request.enabled,
                                ramp_frames: 480,
                            },
                        )
                        .map_err(|_| NativeDesktopBackendError::Internal),
                    DeviceClass::Camera if optional.selection().camera => {
                        camera_enabled = request.enabled;
                        if !camera_enabled {
                            telemetry.camera_active.store(false, Ordering::Release);
                        }
                        Ok(())
                    }
                    DeviceClass::Display
                    | DeviceClass::Microphone
                    | DeviceClass::SystemAudio
                    | DeviceClass::Camera => Err(NativeDesktopBackendError::TargetUnavailable),
                };
                let _ = reply.send(result);
                if result.is_err() {
                    continue;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        let mut made_progress = false;
        match source.poll_frame() {
            Ok(Some(frame)) => {
                made_progress = true;
                if !paused {
                    match screen_clock.normalize(frame) {
                        Ok(Some((sequence, timestamp, pixels))) => {
                            if let Err(error) = push_normalized_screen(
                                &mut recording,
                                studio.as_mut(),
                                sequence,
                                timestamp,
                                pixels,
                            ) {
                                let outcome = fail_optional_worker(
                                    &mut source,
                                    &mut optional,
                                    recording,
                                    studio,
                                    &telemetry,
                                    error,
                                );
                                return WorkerCompletion { outcome };
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let outcome = fail_optional_worker(
                                &mut source,
                                &mut optional,
                                recording,
                                studio,
                                &telemetry,
                                error,
                            );
                            return WorkerCompletion { outcome };
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                let outcome = fail_optional_worker(
                    &mut source,
                    &mut optional,
                    recording,
                    studio,
                    &telemetry,
                    map_capture_error(error),
                );
                return WorkerCompletion { outcome };
            }
        }

        match optional.poll() {
            Ok(report) if report.termination.is_none() => {
                made_progress |= !report.output_samples.is_empty();
                record_optional_diagnostics(&report.diagnostics, &telemetry);
                if let Err(error) = push_optional_outputs(
                    &mut recording,
                    studio.as_mut(),
                    report.output_samples,
                    &telemetry,
                    camera_enabled,
                ) {
                    let outcome = fail_optional_worker(
                        &mut source,
                        &mut optional,
                        recording,
                        studio,
                        &telemetry,
                        error,
                    );
                    return WorkerCompletion { outcome };
                }
            }
            Ok(report) => {
                record_optional_diagnostics(&report.diagnostics, &telemetry);
                let _ = push_optional_outputs(
                    &mut recording,
                    studio.as_mut(),
                    report.output_samples,
                    &telemetry,
                    camera_enabled,
                );
                let termination = report
                    .termination
                    .expect("guard: terminal optional-input report");
                let outcome = fail_optional_worker_after_optional_terminal(
                    &mut source,
                    recording,
                    studio,
                    &telemetry,
                    termination_confirmed(&termination),
                );
                return WorkerCompletion { outcome };
            }
            Err(_) => {
                let outcome = fail_optional_worker(
                    &mut source,
                    &mut optional,
                    recording,
                    studio,
                    &telemetry,
                    NativeDesktopBackendError::Internal,
                );
                return WorkerCompletion { outcome };
            }
        }
        if !made_progress {
            thread::park_timeout(WORKER_IDLE_POLL);
        }
    }
}

fn record_optional_diagnostics(diagnostics: &[AvDiagnostic], telemetry: &OptionalWorkerTelemetry) {
    for diagnostic in diagnostics {
        if let Some(class) = diagnostic.class {
            match class {
                AvSourceClass::Microphone => {
                    telemetry.microphone_meter.store(0, Ordering::Release);
                }
                AvSourceClass::SystemAudio => {
                    telemetry.system_audio_meter.store(0, Ordering::Release);
                }
                AvSourceClass::Camera => {
                    telemetry.camera_active.store(false, Ordering::Release);
                }
            }
        }
        eprintln!(
            "Frame optional-input diagnostic: class={:?} route={:?} capability={:?} timing={:?} code={:?}",
            diagnostic.class,
            diagnostic.route,
            diagnostic.capability,
            diagnostic.timing,
            diagnostic.code
        );
    }
}

fn push_normalized_screen(
    recording: &mut PendingRecordingGraph,
    studio: Option<&mut DesktopStudioRecording>,
    sequence: u64,
    timestamp: FrameTimestamp,
    pixels: Vec<u8>,
) -> Result<(), NativeDesktopBackendError> {
    if let Some(studio) = studio {
        studio.push_screen(sequence, timestamp, pixels.clone())?;
    }
    let frame = BgraScreenFrame::new(sequence, timestamp, pixels).map_err(map_recording_error)?;
    match recording {
        PendingRecordingGraph::ScreenOnly(recording) => recording.push_frame(frame).map(drop),
        PendingRecordingGraph::ScreenAudio(recording) => {
            recording.push_video_frame(frame).map(drop)
        }
    }
    .map_err(map_recording_error)
}

fn push_optional_outputs(
    recording: &mut PendingRecordingGraph,
    mut studio: Option<&mut DesktopStudioRecording>,
    samples: Vec<NativeAvGraphOutputSample>,
    telemetry: &OptionalWorkerTelemetry,
    camera_enabled: bool,
) -> Result<(), NativeDesktopBackendError> {
    for sample in samples {
        let kind = sample.kind();
        let sequence = sample.sequence();
        let timestamp = sample.timestamp();
        let bytes = sample.into_bytes();
        match kind {
            NativeAvGraphOutputKind::MixedAudio => {
                let PendingRecordingGraph::ScreenAudio(recording) = recording else {
                    return Err(NativeDesktopBackendError::Internal);
                };
                recording
                    .push_audio_chunk(
                        F32StereoAudioChunk::new(
                            sequence,
                            timestamp.pts_ns,
                            timestamp.duration_ns,
                            timestamp.discontinuity,
                            bytes,
                        )
                        .map_err(map_recording_error)?,
                    )
                    .map(drop)
                    .map_err(map_recording_error)?;
            }
            NativeAvGraphOutputKind::MicrophoneRecord => {
                telemetry
                    .microphone_meter
                    .store(audio_level_basis_points(&bytes), Ordering::Release);
                if let Some(studio) = studio.as_deref_mut() {
                    studio.push_microphone(sequence, timestamp, bytes)?;
                }
            }
            NativeAvGraphOutputKind::SystemAudioRecord => {
                telemetry
                    .system_audio_meter
                    .store(audio_level_basis_points(&bytes), Ordering::Release);
                if let Some(studio) = studio.as_deref_mut() {
                    studio.push_system_audio(sequence, timestamp, bytes)?;
                }
            }
            NativeAvGraphOutputKind::CameraRecord => {
                telemetry
                    .camera_active
                    .store(camera_enabled, Ordering::Release);
                if camera_enabled && let Some(studio) = studio.as_deref_mut() {
                    studio.push_camera(sequence, timestamp, bytes)?;
                }
            }
        }
    }
    Ok(())
}

fn finish_optional_worker(
    source: &mut MacOsScreenCaptureSource,
    optional: &mut MacOsOptionalInputRecording,
    mut recording: PendingRecordingGraph,
    mut studio: Option<DesktopStudioRecording>,
    screen_clock: &mut SharedScreenClock,
    telemetry: &OptionalWorkerTelemetry,
    camera_enabled: bool,
) -> WorkerOutcome {
    let (screen_tail, screen_teardown_confirmed, screen_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    if let Some(error) = screen_error {
        return fail_optional_recorders(
            optional.cancel().is_ok() && screen_teardown_confirmed,
            recording,
            studio,
            error,
        );
    }
    for frame in screen_tail {
        match screen_clock.normalize(frame) {
            Ok(Some((sequence, timestamp, pixels))) => {
                if let Err(error) = push_normalized_screen(
                    &mut recording,
                    studio.as_mut(),
                    sequence,
                    timestamp,
                    pixels,
                ) {
                    return fail_optional_recorders(
                        optional.cancel().is_ok(),
                        recording,
                        studio,
                        error,
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                return fail_optional_recorders(
                    optional.cancel().is_ok(),
                    recording,
                    studio,
                    error,
                );
            }
        }
    }
    if optional.request_stop().is_err() {
        return fail_optional_recorders(
            false,
            recording,
            studio,
            NativeDesktopBackendError::Internal,
        );
    }
    let deadline = Instant::now() + OPTIONAL_STOP_TIMEOUT;
    let optional_teardown_confirmed = loop {
        match optional.poll() {
            Ok(report) => {
                record_optional_diagnostics(&report.diagnostics, telemetry);
                if let Err(error) = push_optional_outputs(
                    &mut recording,
                    studio.as_mut(),
                    report.output_samples,
                    telemetry,
                    camera_enabled,
                ) {
                    return fail_optional_recorders(
                        optional.cancel().is_ok(),
                        recording,
                        studio,
                        error,
                    );
                }
                if let Some(termination) = report.termination {
                    break termination.outcome == NativeAvRuntimeOutcome::Completed
                        && termination_confirmed(&termination);
                }
            }
            Err(_) => break false,
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::park_timeout(WORKER_IDLE_POLL);
    };
    if !optional_teardown_confirmed {
        return fail_optional_recorders(
            false,
            recording,
            studio,
            NativeDesktopBackendError::Internal,
        );
    }
    if diagnostics_failed(telemetry.screen_diagnostic_baseline, source.diagnostics()) {
        return fail_optional_recorders(
            true,
            recording,
            studio,
            NativeDesktopBackendError::Internal,
        );
    }
    finish_optional_recorders(recording, studio)
}

fn finish_optional_recorders(
    recording: PendingRecordingGraph,
    mut studio: Option<DesktopStudioRecording>,
) -> WorkerOutcome {
    let artifact = match recording {
        PendingRecordingGraph::ScreenOnly(mut recording) => {
            if let Err(error) = recording.end_of_stream() {
                return WorkerOutcome::Failed {
                    error: map_recording_error(error),
                    teardown_confirmed: recording.abort().is_ok(),
                };
            }
            match recording.finish(&CancellationToken::new()) {
                Ok(artifact) => CompletedRecordingArtifact::from(artifact),
                Err(error) => {
                    return WorkerOutcome::Failed {
                        teardown_confirmed: recording_finish_teardown_confirmed(&error),
                        error: map_recording_error(error),
                    };
                }
            }
        }
        PendingRecordingGraph::ScreenAudio(mut recording) => {
            if let Err(error) = recording.end_of_stream() {
                return WorkerOutcome::Failed {
                    error: map_recording_error(error),
                    teardown_confirmed: recording.abort().is_ok(),
                };
            }
            match recording.finish(&CancellationToken::new()) {
                Ok(artifact) => CompletedRecordingArtifact::from(artifact),
                Err(error) => {
                    return WorkerOutcome::Failed {
                        teardown_confirmed: recording_finish_teardown_confirmed(&error),
                        error: map_recording_error(error),
                    };
                }
            }
        }
    };
    match studio
        .take()
        .map(DesktopStudioRecording::finish)
        .transpose()
    {
        Ok(studio) => {
            let mut artifact = artifact;
            artifact.studio_project = studio.map(|artifact| artifact.project);
            WorkerOutcome::Finished(artifact)
        }
        Err(failure) => WorkerOutcome::Failed {
            error: failure.error,
            teardown_confirmed: failure.teardown_confirmed,
        },
    }
}

fn cancel_optional_worker(
    source: &mut MacOsScreenCaptureSource,
    optional: &mut MacOsOptionalInputRecording,
    recording: PendingRecordingGraph,
    studio: Option<DesktopStudioRecording>,
    telemetry: &OptionalWorkerTelemetry,
) -> WorkerOutcome {
    let (_, screen_teardown_confirmed, screen_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    let optional_teardown_confirmed = optional.cancel().is_ok();
    let recorder_teardown_confirmed = abort_pending_recording(recording);
    let studio_teardown_confirmed = abort_studio(studio);
    telemetry.microphone_meter.store(0, Ordering::Release);
    telemetry.system_audio_meter.store(0, Ordering::Release);
    telemetry.camera_active.store(false, Ordering::Release);
    if let Some(error) = screen_error {
        WorkerOutcome::Failed {
            error,
            teardown_confirmed: screen_teardown_confirmed
                && optional_teardown_confirmed
                && recorder_teardown_confirmed
                && studio_teardown_confirmed,
        }
    } else if screen_teardown_confirmed
        && optional_teardown_confirmed
        && recorder_teardown_confirmed
        && studio_teardown_confirmed
    {
        WorkerOutcome::Cancelled
    } else {
        WorkerOutcome::Failed {
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: false,
        }
    }
}

fn fail_optional_worker(
    source: &mut MacOsScreenCaptureSource,
    optional: &mut MacOsOptionalInputRecording,
    recording: PendingRecordingGraph,
    studio: Option<DesktopStudioRecording>,
    telemetry: &OptionalWorkerTelemetry,
    error: NativeDesktopBackendError,
) -> WorkerOutcome {
    let (_, screen_teardown_confirmed, _) = classify_screen_stop(source.stop_and_drain_frames());
    let optional_teardown_confirmed = optional.cancel().is_ok();
    telemetry.microphone_meter.store(0, Ordering::Release);
    telemetry.system_audio_meter.store(0, Ordering::Release);
    telemetry.camera_active.store(false, Ordering::Release);
    fail_optional_recorders(
        screen_teardown_confirmed && optional_teardown_confirmed,
        recording,
        studio,
        error,
    )
}

fn fail_optional_worker_after_optional_terminal(
    source: &mut MacOsScreenCaptureSource,
    recording: PendingRecordingGraph,
    studio: Option<DesktopStudioRecording>,
    telemetry: &OptionalWorkerTelemetry,
    optional_teardown_confirmed: bool,
) -> WorkerOutcome {
    let (_, screen_teardown_confirmed, _) = classify_screen_stop(source.stop_and_drain_frames());
    telemetry.microphone_meter.store(0, Ordering::Release);
    telemetry.system_audio_meter.store(0, Ordering::Release);
    telemetry.camera_active.store(false, Ordering::Release);
    fail_optional_recorders(
        screen_teardown_confirmed && optional_teardown_confirmed,
        recording,
        studio,
        NativeDesktopBackendError::Internal,
    )
}

fn fail_optional_recorders(
    native_teardown_confirmed: bool,
    recording: PendingRecordingGraph,
    studio: Option<DesktopStudioRecording>,
    error: NativeDesktopBackendError,
) -> WorkerOutcome {
    WorkerOutcome::Failed {
        error,
        teardown_confirmed: native_teardown_confirmed
            && abort_pending_recording(recording)
            && abort_studio(studio),
    }
}

fn abort_pending_recording(recording: PendingRecordingGraph) -> bool {
    match recording {
        PendingRecordingGraph::ScreenOnly(recording) => recording.abort().is_ok(),
        PendingRecordingGraph::ScreenAudio(recording) => recording.abort().is_ok(),
    }
}

fn termination_confirmed(termination: &frame_media::NativeAvTermination) -> bool {
    termination.source_teardown == NativeAvSourceTeardown::Confirmed
        && termination.graph_teardown == NativeAvGraphTeardown::NullReached
}

pub(super) fn run_screen_studio_capture_worker(
    mut source: MacOsScreenCaptureSource,
    mut recording: ScreenRecording,
    mut studio: DesktopStudioRecording,
    control: Receiver<WorkerControl>,
    diagnostic_baseline: MacOsCaptureDiagnostics,
) -> WorkerCompletion {
    let mut timestamps = None;
    loop {
        match control.try_recv() {
            Ok(WorkerControl::Stop) => {
                let outcome = finish_screen_studio_recording(
                    &mut source,
                    recording,
                    studio,
                    &mut timestamps,
                    diagnostic_baseline,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Cancel) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let outcome = cancel_screen_studio_recording(
                    &mut source,
                    recording,
                    studio,
                    diagnostic_baseline,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Pause(reply) | WorkerControl::Resume(reply)) => {
                let _ = reply.send(Err(NativeDesktopBackendError::Unavailable));
            }
            Ok(WorkerControl::Input { reply, .. }) => {
                let _ = reply.send(Err(NativeDesktopBackendError::Unavailable));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        match source.poll_frame() {
            Ok(Some(frame)) => {
                if let Err(error) =
                    push_screen_studio_frame(&mut recording, &mut studio, frame, &mut timestamps)
                {
                    let outcome =
                        fail_screen_studio_recording(&mut source, recording, studio, error);
                    return WorkerCompletion { outcome };
                }
            }
            Ok(None) => thread::park_timeout(WORKER_IDLE_POLL),
            Err(error) => {
                let outcome = fail_screen_studio_recording(
                    &mut source,
                    recording,
                    studio,
                    map_capture_error(error),
                );
                return WorkerCompletion { outcome };
            }
        }
    }
}

fn push_screen_studio_frame(
    recording: &mut ScreenRecording,
    studio: &mut DesktopStudioRecording,
    frame: frame_macos_screen_capture::MacOsCaptureFrame,
    timestamps: &mut Option<SharedClockNormalizer>,
) -> Result<(), NativeDesktopBackendError> {
    let sequence = frame.sequence();
    let source_pts_ns = frame
        .source_pts_ns()
        .ok_or(NativeDesktopBackendError::Internal)?;
    let normalizer =
        timestamps.get_or_insert_with(|| SharedClockNormalizer::screen_only(source_pts_ns));
    let timestamp = normalizer
        .normalize_video(source_pts_ns, frame.timestamp())
        .map_err(map_recording_error)?;
    let pixels = frame.into_pixels();
    studio.push_screen(sequence, timestamp, pixels.to_vec())?;
    let frame = BgraScreenFrame::new(sequence, timestamp, pixels).map_err(map_recording_error)?;
    recording
        .push_frame(frame)
        .map(|_| ())
        .map_err(map_recording_error)
}

fn finish_screen_studio_recording(
    source: &mut MacOsScreenCaptureSource,
    mut recording: ScreenRecording,
    mut studio: DesktopStudioRecording,
    timestamps: &mut Option<SharedClockNormalizer>,
    diagnostic_baseline: MacOsCaptureDiagnostics,
) -> WorkerOutcome {
    let (tail, teardown_confirmed, stop_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    if let Some(error) = stop_error {
        return fail_screen_studio_after_native_stop(recording, studio, error, teardown_confirmed);
    }
    for frame in tail {
        if let Err(error) = push_screen_studio_frame(&mut recording, &mut studio, frame, timestamps)
        {
            return fail_screen_studio_after_native_stop(
                recording,
                studio,
                error,
                teardown_confirmed,
            );
        }
    }
    if diagnostics_failed(diagnostic_baseline, source.diagnostics()) {
        return fail_screen_studio_after_native_stop(
            recording,
            studio,
            NativeDesktopBackendError::Internal,
            teardown_confirmed,
        );
    }
    if let Err(error) = recording.end_of_stream() {
        return fail_screen_studio_after_native_stop(
            recording,
            studio,
            map_recording_error(error),
            teardown_confirmed,
        );
    }
    match recording.finish(&CancellationToken::new()) {
        Ok(artifact) => match studio.finish() {
            Ok(studio) => {
                let mut completed = CompletedRecordingArtifact::from(artifact);
                completed.studio_project = Some(studio.project);
                WorkerOutcome::Finished(completed)
            }
            Err(failure) => WorkerOutcome::Failed {
                error: failure.error,
                teardown_confirmed: failure.teardown_confirmed,
            },
        },
        Err(error) => {
            let recording_teardown_confirmed = recording_finish_teardown_confirmed(&error);
            WorkerOutcome::Failed {
                error: map_recording_error(error),
                teardown_confirmed: teardown_confirmed
                    && recording_teardown_confirmed
                    && studio.abort().is_ok(),
            }
        }
    }
}

fn cancel_screen_studio_recording(
    source: &mut MacOsScreenCaptureSource,
    recording: ScreenRecording,
    studio: DesktopStudioRecording,
    diagnostic_baseline: MacOsCaptureDiagnostics,
) -> WorkerOutcome {
    let (_, capture_teardown_confirmed, capture_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    let recording_teardown_confirmed = recording.abort().is_ok();
    let studio_teardown_confirmed = studio.abort().is_ok();
    if let Some(error) = capture_error {
        return WorkerOutcome::Failed {
            error,
            teardown_confirmed: capture_teardown_confirmed
                && recording_teardown_confirmed
                && studio_teardown_confirmed,
        };
    }
    if capture_teardown_confirmed
        && recording_teardown_confirmed
        && studio_teardown_confirmed
        && !diagnostics_failed(diagnostic_baseline, source.diagnostics())
    {
        WorkerOutcome::Cancelled
    } else {
        WorkerOutcome::Failed {
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: capture_teardown_confirmed
                && recording_teardown_confirmed
                && studio_teardown_confirmed,
        }
    }
}

fn fail_screen_studio_recording(
    source: &mut MacOsScreenCaptureSource,
    recording: ScreenRecording,
    studio: DesktopStudioRecording,
    primary_error: NativeDesktopBackendError,
) -> WorkerOutcome {
    let (_, capture_teardown_confirmed, _) = classify_screen_stop(source.stop_and_drain_frames());
    fail_screen_studio_after_native_stop(
        recording,
        studio,
        primary_error,
        capture_teardown_confirmed,
    )
}

fn fail_screen_studio_after_native_stop(
    recording: ScreenRecording,
    studio: DesktopStudioRecording,
    primary_error: NativeDesktopBackendError,
    native_teardown_confirmed: bool,
) -> WorkerOutcome {
    WorkerOutcome::Failed {
        error: primary_error,
        teardown_confirmed: native_teardown_confirmed
            && recording.abort().is_ok()
            && studio.abort().is_ok(),
    }
}

fn normalize_timestamp(
    source_origin_ns: u64,
    last_output_end_ns: &mut u64,
    source_pts_ns: u64,
    duration_ns: u64,
    source_discontinuity: bool,
) -> Result<FrameTimestamp, ScreenRecordingError> {
    let candidate_pts_ns = source_pts_ns
        .checked_sub(source_origin_ns)
        .ok_or(ScreenRecordingError::NonMonotonicFrame)?;
    let output_pts_ns = candidate_pts_ns.max(*last_output_end_ns);
    let mut timestamp = FrameTimestamp::new(output_pts_ns, duration_ns)
        .map_err(|_| ScreenRecordingError::InvalidFrame)?;
    timestamp.discontinuity = source_discontinuity || output_pts_ns != candidate_pts_ns;
    *last_output_end_ns = timestamp.end_ns();
    Ok(timestamp)
}

pub(super) fn run_av_capture_worker(
    mut source: MacOsScreenCaptureSource,
    mut system_audio: MacOsSystemAudioSource,
    mut recording: ScreenAudioRecording,
    mut studio: Option<DesktopStudioRecording>,
    mut timestamps: SharedClockNormalizer,
    control: Receiver<WorkerControl>,
    telemetry: AvWorkerTelemetry,
) -> WorkerCompletion {
    let mut poll_audio_first = false;
    loop {
        match control.try_recv() {
            Ok(WorkerControl::Stop) => {
                let outcome = finish_av_worker_recording(
                    &mut source,
                    &mut system_audio,
                    recording,
                    studio,
                    &mut timestamps,
                    telemetry.screen_diagnostic_baseline,
                    telemetry.audio_diagnostic_baseline,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Cancel) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let outcome = cancel_av_worker_recording(
                    &mut source,
                    &mut system_audio,
                    recording,
                    studio,
                    telemetry.screen_diagnostic_baseline,
                    telemetry.audio_diagnostic_baseline,
                );
                return WorkerCompletion { outcome };
            }
            Ok(WorkerControl::Pause(reply) | WorkerControl::Resume(reply)) => {
                let _ = reply.send(Err(NativeDesktopBackendError::Unavailable));
            }
            Ok(WorkerControl::Input { reply, .. }) => {
                let _ = reply.send(Err(NativeDesktopBackendError::Unavailable));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        // Alternate which source is polled first, but admit at most one item
        // from each source per turn. Neither callback queue can starve the
        // other, and control is observed before every bounded pair.
        let first = if poll_audio_first {
            poll_audio(
                &mut system_audio,
                &mut recording,
                &mut timestamps,
                studio.as_mut(),
                &telemetry.system_audio_meter,
            )
        } else {
            poll_screen(
                &mut source,
                &mut recording,
                &mut timestamps,
                studio.as_mut(),
            )
        };
        let first_did_work = match first {
            Ok(did_work) => did_work,
            Err(error) => {
                let outcome = fail_worker_recording(
                    &mut source,
                    &mut system_audio,
                    recording,
                    studio,
                    telemetry.screen_diagnostic_baseline,
                    telemetry.audio_diagnostic_baseline,
                    error,
                );
                return WorkerCompletion { outcome };
            }
        };
        let second = if poll_audio_first {
            poll_screen(
                &mut source,
                &mut recording,
                &mut timestamps,
                studio.as_mut(),
            )
        } else {
            poll_audio(
                &mut system_audio,
                &mut recording,
                &mut timestamps,
                studio.as_mut(),
                &telemetry.system_audio_meter,
            )
        };
        let second_did_work = match second {
            Ok(did_work) => did_work,
            Err(error) => {
                let outcome = fail_worker_recording(
                    &mut source,
                    &mut system_audio,
                    recording,
                    studio,
                    telemetry.screen_diagnostic_baseline,
                    telemetry.audio_diagnostic_baseline,
                    error,
                );
                return WorkerCompletion { outcome };
            }
        };
        poll_audio_first = !poll_audio_first;
        if !first_did_work && !second_did_work {
            thread::park_timeout(WORKER_IDLE_POLL);
        }
    }
}

pub(super) fn calibrate_av_startup(
    source: &mut MacOsScreenCaptureSource,
    system_audio: &mut MacOsSystemAudioSource,
    recording: &mut ScreenAudioRecording,
    mut studio: Option<&mut DesktopStudioRecording>,
) -> Result<SharedClockNormalizer, NativeDesktopBackendError> {
    let deadline = Instant::now()
        .checked_add(STARTUP_CALIBRATION_TIMEOUT)
        .ok_or(NativeDesktopBackendError::Internal)?;
    let mut screen = None;
    let mut audio = None;
    loop {
        if screen.is_none() {
            screen = source.poll_frame().map_err(map_capture_error)?;
            if screen
                .as_ref()
                .is_some_and(|frame| frame.source_pts_ns().is_none())
            {
                return Err(NativeDesktopBackendError::Internal);
            }
        }
        if audio.is_none() {
            audio = system_audio.poll_chunk().map_err(map_system_audio_error)?;
        }
        if screen.is_some() && audio.is_some() {
            let screen = screen.take().ok_or(NativeDesktopBackendError::Internal)?;
            let audio = audio.take().ok_or(NativeDesktopBackendError::Internal)?;
            let screen_source_pts_ns = screen
                .source_pts_ns()
                .ok_or(NativeDesktopBackendError::Internal)?;
            let audio_source_pts_ns = audio.source_pts_ns();
            let mut timestamps =
                SharedClockNormalizer::new(screen_source_pts_ns, audio_source_pts_ns);
            if screen_source_pts_ns <= audio_source_pts_ns {
                push_capture_frame(recording, screen, &mut timestamps, studio.as_deref_mut())?;
                push_audio_chunk(recording, audio, &mut timestamps, studio.as_deref_mut())?;
            } else {
                push_audio_chunk(recording, audio, &mut timestamps, studio.as_deref_mut())?;
                push_capture_frame(recording, screen, &mut timestamps, studio.as_deref_mut())?;
            }
            return Ok(timestamps);
        }

        let now = Instant::now();
        if now >= deadline {
            // Do not fabricate PCM or silently relabel this graph as A/V. The
            // caller can retry with system audio disabled and get screen-only.
            return Err(NativeDesktopBackendError::Unavailable);
        }
        thread::park_timeout(WORKER_IDLE_POLL.min(deadline.saturating_duration_since(now)));
    }
}

fn poll_screen(
    source: &mut MacOsScreenCaptureSource,
    recording: &mut ScreenAudioRecording,
    timestamps: &mut SharedClockNormalizer,
    studio: Option<&mut DesktopStudioRecording>,
) -> Result<bool, NativeDesktopBackendError> {
    match source.poll_frame() {
        Ok(Some(frame)) => {
            push_capture_frame(recording, frame, timestamps, studio)?;
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(error) => Err(map_capture_error(error)),
    }
}

fn poll_audio(
    system_audio: &mut MacOsSystemAudioSource,
    recording: &mut ScreenAudioRecording,
    timestamps: &mut SharedClockNormalizer,
    studio: Option<&mut DesktopStudioRecording>,
    system_audio_meter: &AtomicU16,
) -> Result<bool, NativeDesktopBackendError> {
    match system_audio.poll_chunk() {
        Ok(Some(chunk)) => {
            system_audio_meter.store(
                audio_level_basis_points(chunk.samples_f32le()),
                Ordering::Release,
            );
            push_audio_chunk(recording, chunk, timestamps, studio)?;
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(error) => Err(map_system_audio_error(error)),
    }
}

fn audio_level_basis_points(samples_f32le: &[u8]) -> u16 {
    let peak = samples_f32le
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]).abs())
        .fold(0.0_f32, f32::max)
        .clamp(0.0, 1.0);
    (peak * 10_000.0).round() as u16
}

fn push_capture_frame(
    recording: &mut ScreenAudioRecording,
    frame: frame_macos_screen_capture::MacOsCaptureFrame,
    timestamps: &mut SharedClockNormalizer,
    studio: Option<&mut DesktopStudioRecording>,
) -> Result<(), NativeDesktopBackendError> {
    let sequence = frame.sequence();
    let source_pts_ns = frame
        .source_pts_ns()
        .ok_or(NativeDesktopBackendError::Internal)?;
    let timestamp = timestamps
        .normalize_video(source_pts_ns, frame.timestamp())
        .map_err(map_recording_error)?;
    let pixels = frame.into_pixels();
    if let Some(studio) = studio {
        studio.push_screen(sequence, timestamp, pixels.to_vec())?;
    }
    let frame = BgraScreenFrame::new(sequence, timestamp, pixels).map_err(map_recording_error)?;
    recording
        .push_video_frame(frame)
        .map(|_| ())
        .map_err(map_recording_error)
}

fn push_audio_chunk(
    recording: &mut ScreenAudioRecording,
    chunk: MacOsSystemAudioChunk,
    timestamps: &mut SharedClockNormalizer,
    studio: Option<&mut DesktopStudioRecording>,
) -> Result<(), NativeDesktopBackendError> {
    let studio_samples = studio.as_ref().map(|_| chunk.samples_f32le().to_vec());
    let chunk = timestamps
        .normalize_audio(chunk)
        .map_err(map_recording_error)?;
    if let (Some(studio), Some(samples)) = (studio, studio_samples) {
        studio.push_system_audio(chunk.sequence(), chunk.timestamp(), samples)?;
    }
    recording
        .push_audio_chunk(chunk)
        .map(|_| ())
        .map_err(map_recording_error)
}

fn finish_av_worker_recording(
    source: &mut MacOsScreenCaptureSource,
    system_audio: &mut MacOsSystemAudioSource,
    mut recording: ScreenAudioRecording,
    mut studio: Option<DesktopStudioRecording>,
    timestamps: &mut SharedClockNormalizer,
    screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    audio_diagnostic_baseline: MacOsSystemAudioDiagnostics,
) -> WorkerOutcome {
    // Stop both native producers before admitting either bounded tail. EOS is
    // serialized only after every accepted tail item reaches its appsrc.
    let (screen_tail, screen_teardown_confirmed, screen_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    let (audio_tail, audio_teardown_confirmed, audio_error) =
        classify_audio_stop(system_audio.stop_and_drain_chunks());
    if let Some(primary_error) = screen_error.or(audio_error) {
        return fail_recorder_after_native_stop(
            recording,
            studio,
            primary_error,
            all_av_teardown_confirmed(screen_teardown_confirmed, audio_teardown_confirmed, true),
        );
    }
    for frame in screen_tail {
        if let Err(error) = push_capture_frame(&mut recording, frame, timestamps, studio.as_mut()) {
            return fail_recorder_after_native_stop(recording, studio, error, true);
        }
    }
    for chunk in audio_tail {
        if let Err(error) = push_audio_chunk(&mut recording, chunk, timestamps, studio.as_mut()) {
            return fail_recorder_after_native_stop(recording, studio, error, true);
        }
    }
    if diagnostics_failed(screen_diagnostic_baseline, source.diagnostics())
        || system_audio_diagnostics_failed(audio_diagnostic_baseline, system_audio.diagnostics())
    {
        return fail_recorder_after_native_stop(
            recording,
            studio,
            NativeDesktopBackendError::Internal,
            true,
        );
    }
    if let Err(error) = recording.end_of_stream() {
        return fail_recorder_after_native_stop(
            recording,
            studio,
            map_recording_error(error),
            true,
        );
    }
    match recording.finish(&CancellationToken::new()) {
        Ok(artifact) => match studio
            .take()
            .map(DesktopStudioRecording::finish)
            .transpose()
        {
            Ok(studio) => {
                let mut completed = CompletedRecordingArtifact::from(artifact);
                completed.studio_project = studio.map(|artifact| artifact.project);
                WorkerOutcome::Finished(completed)
            }
            Err(failure) => WorkerOutcome::Failed {
                error: failure.error,
                teardown_confirmed: failure.teardown_confirmed,
            },
        },
        Err(error) => WorkerOutcome::Failed {
            teardown_confirmed: recording_finish_teardown_confirmed(&error),
            error: map_recording_error(error),
        },
    }
}

fn cancel_av_worker_recording(
    source: &mut MacOsScreenCaptureSource,
    system_audio: &mut MacOsSystemAudioSource,
    recording: ScreenAudioRecording,
    studio: Option<DesktopStudioRecording>,
    screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    audio_diagnostic_baseline: MacOsSystemAudioDiagnostics,
) -> WorkerOutcome {
    let (_, screen_teardown_confirmed, screen_error) =
        classify_screen_stop(source.stop_and_drain_frames());
    let (_, audio_teardown_confirmed, audio_error) =
        classify_audio_stop(system_audio.stop_and_drain_chunks());
    let recording_stopped = recording.abort();
    let studio_stopped = abort_studio(studio);
    if let Some(primary_error) = screen_error.or(audio_error) {
        return WorkerOutcome::Failed {
            error: primary_error,
            teardown_confirmed: all_av_teardown_confirmed(
                screen_teardown_confirmed,
                audio_teardown_confirmed,
                recording_stopped.is_ok() && studio_stopped,
            ),
        };
    }
    if let Err(error) = recording_stopped {
        return WorkerOutcome::Failed {
            error: map_recording_error(error),
            teardown_confirmed: false,
        };
    }
    if !studio_stopped {
        return WorkerOutcome::Failed {
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: false,
        };
    }
    if diagnostics_failed(screen_diagnostic_baseline, source.diagnostics())
        || system_audio_diagnostics_failed(audio_diagnostic_baseline, system_audio.diagnostics())
    {
        WorkerOutcome::Failed {
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: true,
        }
    } else {
        WorkerOutcome::Cancelled
    }
}

fn fail_worker_recording(
    source: &mut MacOsScreenCaptureSource,
    system_audio: &mut MacOsSystemAudioSource,
    recording: ScreenAudioRecording,
    studio: Option<DesktopStudioRecording>,
    screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    audio_diagnostic_baseline: MacOsSystemAudioDiagnostics,
    primary_error: NativeDesktopBackendError,
) -> WorkerOutcome {
    let (_, screen_teardown_confirmed, _) = classify_screen_stop(source.stop_and_drain_frames());
    let (_, audio_teardown_confirmed, _) =
        classify_audio_stop(system_audio.stop_and_drain_chunks());
    if diagnostic_delta(screen_diagnostic_baseline, source.diagnostics()).is_err()
        || system_audio_diagnostic_delta(audio_diagnostic_baseline, system_audio.diagnostics())
            .is_err()
    {
        eprintln!("Frame native A/V diagnostics regressed while preserving the primary failure");
    }
    let recording_teardown_confirmed = recording.abort().is_ok();
    let studio_teardown_confirmed = abort_studio(studio);
    WorkerOutcome::Failed {
        error: primary_error,
        teardown_confirmed: all_av_teardown_confirmed(
            screen_teardown_confirmed,
            audio_teardown_confirmed,
            recording_teardown_confirmed && studio_teardown_confirmed,
        ),
    }
}

fn fail_recorder_after_native_stop(
    recording: ScreenAudioRecording,
    studio: Option<DesktopStudioRecording>,
    primary_error: NativeDesktopBackendError,
    native_teardown_confirmed: bool,
) -> WorkerOutcome {
    WorkerOutcome::Failed {
        error: primary_error,
        teardown_confirmed: native_teardown_confirmed
            && recording.abort().is_ok()
            && abort_studio(studio),
    }
}

fn abort_studio(studio: Option<DesktopStudioRecording>) -> bool {
    studio.is_none_or(|studio| studio.abort().is_ok())
}

pub(super) fn classify_screen_stop(
    result: Result<Vec<frame_macos_screen_capture::MacOsCaptureFrame>, MacOsCaptureStopError>,
) -> (
    Vec<frame_macos_screen_capture::MacOsCaptureFrame>,
    bool,
    Option<NativeDesktopBackendError>,
) {
    match result {
        Ok(tail) => (tail, true, None),
        Err(error) => {
            let teardown_confirmed = error.capture_teardown_confirmed();
            (
                Vec::new(),
                teardown_confirmed,
                Some(map_capture_error(error.into_capture_error())),
            )
        }
    }
}

pub(super) fn classify_audio_stop(
    result: Result<Vec<MacOsSystemAudioChunk>, MacOsSystemAudioStopError>,
) -> (
    Vec<MacOsSystemAudioChunk>,
    bool,
    Option<NativeDesktopBackendError>,
) {
    match result {
        Ok(tail) => (tail, true, None),
        Err(error) => (
            Vec::new(),
            error.capture_teardown_confirmed(),
            Some(map_system_audio_error(error.capture_error())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_clock_preserves_startup_offset_and_clamps_only_per_source_overlap() {
        let mut normalizer = SharedClockNormalizer::new(10_000, 25_000);
        let video = normalizer
            .normalize_video(
                10_000,
                FrameTimestamp::new(0, 1_000).expect("video source timestamp"),
            )
            .expect("normalized video");
        let mut audio_end = 0;
        let audio = normalize_timestamp(
            normalizer.source_origin_ns,
            &mut audio_end,
            25_000,
            1_000,
            false,
        )
        .expect("normalized audio");
        let overlapping_video = normalizer
            .normalize_video(
                10_500,
                FrameTimestamp::new(500, 1_000).expect("overlapping source timestamp"),
            )
            .expect("clamped video overlap");

        assert_eq!(video.pts_ns, 0);
        assert_eq!(audio.pts_ns, 15_000);
        assert_eq!(overlapping_video.pts_ns, video.end_ns());
        assert!(overlapping_video.discontinuity);
        assert_eq!(audio_end, 16_000);
    }

    #[test]
    fn shared_clock_rejects_a_sample_before_the_calibrated_origin() {
        let mut end = 0;
        assert!(matches!(
            normalize_timestamp(10_000, &mut end, 9_999, 1_000, false),
            Err(ScreenRecordingError::NonMonotonicFrame)
        ));
    }

    #[test]
    fn audio_meter_is_bounded_and_uses_no_raw_payload() {
        let samples: Vec<_> = [0.0_f32, -0.25, 0.75, 1.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(audio_level_basis_points(&samples), 10_000);
    }
}
