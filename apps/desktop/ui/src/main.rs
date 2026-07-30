#[cfg(all(target_arch = "wasm32", feature = "csr"))]
mod browser {
    mod region_picker;

    use std::{
        collections::HashMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use frame_client::{InstantUiPhaseV1, InstantUiProgressV1};
    use frame_desktop_core::{
        AudioMeterSnapshot, BackendEvent, CAPTURE_ARTIFACT_SUMMARY_VERSION,
        CAPTURE_TARGET_CATALOG_VERSION, CameraPreviewState, CaptureTargetKind, CommandOutcome,
        DESKTOP_INPUT_TELEMETRY_INTERVAL_MS, DESKTOP_RUNTIME_VERSION, DesktopAdapterKind,
        DesktopBootstrap, DesktopDispatch, DesktopRuntimeEvent, DesktopRuntimeSnapshot,
        DesktopWindowContext, DeviceClass, DeviceState, EditorMutation, EditorState, ExportProfile,
        ExportState, IPC_PROTOCOL_VERSION, InstantFinalizeCapabilityState,
        InstantFinalizeCommandV1, InstantFinalizeHandle, InstantFinalizeUiUpdate, IpcCommand,
        LegacyProjectCatalogAvailability, LegacyProjectStatus, LifecycleAction,
        NativeStudioPreviewOutcome, NativeStudioPreviewRequest, NativeStudioProjectStatus,
        PublicErrorCode, RecorderAdapterState, RecorderMode, RecorderState, RequestEnvelope,
        RequestId, ShellCapabilities, UpdateAction, UpdateState, UploadState, WindowRole,
        instant_error_message, instant_progress_announcement,
    };
    use frame_ui::{
        Alert, Badge, BadgeVariant, Button, ButtonGroup, ButtonVariant, Card, CardFrame,
        DialogContent, DialogOverlay, FieldGroup, Input, Label, Meter, NavigationMenu, Progress,
        ToggleGroup, UiStyles,
    };
    use js_sys::Reflect;
    use leptos::prelude::*;
    use serde::Serialize;
    use wasm_bindgen::{Clamped, JsCast, prelude::*};
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

    use self::region_picker::RegionPicker;

    const RECORDER_POLL_INTERVAL: Duration =
        Duration::from_millis(DESKTOP_INPUT_TELEMETRY_INTERVAL_MS);

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(
            catch,
            js_namespace = ["window", "__TAURI__", "core"],
            js_name = invoke
        )]
        async fn invoke_without_args(command: &str) -> Result<JsValue, JsValue>;

        #[wasm_bindgen(
            catch,
            js_namespace = ["window", "__TAURI__", "core"],
            js_name = invoke
        )]
        async fn invoke_with_args(command: &str, args: JsValue) -> Result<JsValue, JsValue>;
    }

    #[derive(Serialize)]
    struct DispatchArgs<'a> {
        #[serde(rename = "requestJson")]
        request_json: &'a str,
    }

    #[derive(Serialize)]
    struct InstantFinalizeArgs<'a> {
        #[serde(rename = "commandJson")]
        command_json: &'a str,
    }

    #[derive(Clone)]
    struct DesktopClient {
        contexts: Arc<Vec<DesktopWindowContext>>,
        sequences: Arc<Mutex<HashMap<WindowRole, u64>>>,
        next_identifier: Arc<AtomicU64>,
        instant_next_sequence: Arc<AtomicU64>,
    }

    impl DesktopClient {
        fn new(contexts: Vec<DesktopWindowContext>, instant_next_sequence: Option<u64>) -> Self {
            Self {
                contexts: Arc::new(contexts),
                sequences: Arc::new(Mutex::new(HashMap::new())),
                next_identifier: Arc::new(AtomicU64::new(0)),
                instant_next_sequence: Arc::new(AtomicU64::new(instant_next_sequence.unwrap_or(0))),
            }
        }

        fn next_intent_id(&self) -> String {
            format!("ui-intent-{:016x}", self.next_identifier())
        }

        fn next_identifier(&self) -> u64 {
            self.next_identifier
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_or(u64::MAX, |previous| previous + 1)
        }

        async fn dispatch(
            &self,
            role: WindowRole,
            command: IpcCommand,
        ) -> Result<DesktopDispatch, ()> {
            let context = self
                .contexts
                .iter()
                .find(|context| context.role == role)
                .ok_or(())?;
            let sequence = {
                let mut sequences = self.sequences.lock().map_err(|_| ())?;
                let sequence = sequences.entry(role).or_insert(0);
                *sequence = sequence.checked_add(1).ok_or(())?;
                *sequence
            };
            let request = RequestEnvelope {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: RequestId::new(format!("ui-request-{:016x}", self.next_identifier()))
                    .map_err(|_| ())?,
                window_id: context.window_id.clone(),
                session_id: context.session_id.clone(),
                sequence,
                command,
            };
            let request_json = serde_json::to_string(&request).map_err(|_| ())?;
            let args = serde_wasm_bindgen::to_value(&DispatchArgs {
                request_json: &request_json,
            })
            .map_err(|_| ())?;
            let value = invoke_with_args("dispatch_main", args)
                .await
                .map_err(|_| ())?;
            serde_wasm_bindgen::from_value(value).map_err(|_| ())
        }

        async fn finalize_instant(
            &self,
            handle: InstantFinalizeHandle,
        ) -> Result<InstantFinalizeUiUpdate, ()> {
            let sequence = self.instant_next_sequence.load(Ordering::Relaxed);
            if sequence == 0 {
                return Err(());
            }
            let command = InstantFinalizeCommandV1::new(handle, sequence).map_err(|_| ())?;
            let command_json = serde_json::to_string(&command).map_err(|_| ())?;
            let args = serde_wasm_bindgen::to_value(&InstantFinalizeArgs {
                command_json: &command_json,
            })
            .map_err(|_| ())?;
            let value = invoke_with_args("finalize_instant", args)
                .await
                .map_err(|_| ())?;
            let update: InstantFinalizeUiUpdate =
                serde_wasm_bindgen::from_value(value).map_err(|_| ())?;
            if update.runtime_version != DESKTOP_RUNTIME_VERSION
                || update.command_protocol_version
                    != frame_desktop_core::INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION
                || update.command_sequence != sequence
                || update.progress.validate().is_err()
            {
                return Err(());
            }
            let next_sequence = if matches!(
                update.progress.phase,
                InstantUiPhaseV1::ShareReady
                    | InstantUiPhaseV1::Cancelled
                    | InstantUiPhaseV1::RecoveryRequired
            ) {
                0
            } else {
                sequence.checked_add(1).ok_or(())?
            };
            self.instant_next_sequence
                .store(next_sequence, Ordering::Relaxed);
            Ok(update)
        }
    }

    async fn bootstrap_native() -> Result<(ShellCapabilities, DesktopBootstrap), ()> {
        let tauri =
            Reflect::get(&js_sys::global(), &JsValue::from_str("__TAURI__")).map_err(|_| ())?;
        if tauri.is_null() || tauri.is_undefined() {
            return Err(());
        }
        let shell_value = invoke_without_args("bootstrap_main")
            .await
            .map_err(|_| ())?;
        let shell: ShellCapabilities =
            serde_wasm_bindgen::from_value(shell_value).map_err(|_| ())?;
        if !shell.is_current_backend_truth() {
            return Err(());
        }
        let desktop_value = invoke_without_args("bootstrap_desktop")
            .await
            .map_err(|_| ())?;
        let desktop: DesktopBootstrap =
            serde_wasm_bindgen::from_value(desktop_value).map_err(|_| ())?;
        (desktop.runtime_version == DESKTOP_RUNTIME_VERSION
            && desktop.snapshot.version == DESKTOP_RUNTIME_VERSION
            && recorder_adapter_matches(shell.recorder_adapter, desktop.snapshot.adapter)
            && shell.instant_finalize == desktop.snapshot.instant_finalize)
            .then_some((shell, desktop))
            .ok_or(())
    }

    const fn recorder_adapter_matches(
        shell: RecorderAdapterState,
        runtime: DesktopAdapterKind,
    ) -> bool {
        matches!(
            (shell, runtime),
            (
                RecorderAdapterState::Unavailable,
                DesktopAdapterKind::Unavailable
            ) | (
                RecorderAdapterState::DeterministicFake,
                DesktopAdapterKind::DeterministicFake
            ) | (
                RecorderAdapterState::NativeMacOsDisplay,
                DesktopAdapterKind::NativeMacOs
            ) | (
                RecorderAdapterState::NativeWindowsDisplayWindowRegion,
                DesktopAdapterKind::NativeWindows
            )
        )
    }

    fn submit(
        client: RwSignal<Option<DesktopClient>>,
        snapshot: RwSignal<Option<DesktopRuntimeSnapshot>>,
        status: RwSignal<String>,
        error: RwSignal<Option<String>>,
        busy: RwSignal<bool>,
        role: WindowRole,
        command: IpcCommand,
    ) {
        let Some(client) = client.get_untracked() else {
            error.set(Some("The native backend is unavailable.".into()));
            return;
        };
        if busy.get_untracked() {
            return;
        }
        let recorder_poll = matches!(command, IpcCommand::RecorderPoll);
        busy.set(true);
        spawn_local(async move {
            match client.dispatch(role, command).await {
                Ok(dispatch) => {
                    let operation_error = match dispatch.response.outcome {
                        CommandOutcome::Ok { .. } => None,
                        CommandOutcome::Error { code, .. } => Some(public_error(code).into()),
                    };
                    match validated_snapshot(
                        snapshot
                            .get_untracked()
                            .map(|state| (state.meter, state.camera_preview)),
                        recorder_poll,
                        &dispatch,
                    ) {
                        Ok(next) => {
                            status.set(next.announcement.clone());
                            snapshot.set(Some(next));
                            error.set(operation_error);
                        }
                        Err(()) => {
                            status.set("Native telemetry event rejected.".into());
                            error.set(Some(
                                "Frame rejected malformed or stale input telemetry.".into(),
                            ));
                        }
                    }
                }
                Err(()) => {
                    error.set(Some(
                        "The native command boundary rejected the request. No success was assumed."
                            .into(),
                    ));
                    status.set("Native backend unavailable.".into());
                }
            }
            busy.set(false);
        });
    }

    fn submit_preview(
        client: RwSignal<Option<DesktopClient>>,
        snapshot: RwSignal<Option<DesktopRuntimeSnapshot>>,
        status: RwSignal<String>,
        error: RwSignal<Option<String>>,
        busy: RwSignal<bool>,
        preview_summary: RwSignal<String>,
        request: NativeStudioPreviewRequest,
    ) {
        let Some(client) = client.get_untracked() else {
            error.set(Some("The native backend is unavailable.".into()));
            return;
        };
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            let command = IpcCommand::EditorPreview {
                editor_revision: request.editor_revision,
                position_ms: request.position_ms,
            };
            match client.dispatch(WindowRole::Editor, command).await {
                Ok(dispatch) => {
                    let operation_error = match dispatch.response.outcome {
                        CommandOutcome::Ok { .. } => None,
                        CommandOutcome::Error { code, .. } => Some(public_error(code).into()),
                    };
                    let next = validated_snapshot(
                        snapshot
                            .get_untracked()
                            .map(|state| (state.meter, state.camera_preview)),
                        false,
                        &dispatch,
                    );
                    let previews = dispatch
                        .events
                        .iter()
                        .filter_map(|envelope| match &envelope.event {
                            DesktopRuntimeEvent::Backend(BackendEvent::EditorPreviewReady {
                                preview,
                                ..
                            }) if envelope.owner == WindowRole::Editor => Some(preview),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    match (next, operation_error, previews.as_slice()) {
                        (Ok(next), None, [preview])
                            if preview.validate_for(request).is_ok()
                                && draw_studio_preview(preview).is_ok() =>
                        {
                            preview_summary.set(format!(
                                "Preview at {} ms maps to source {} ms. Microphone {}; system audio {}.",
                                preview.position_ms,
                                preview.source_position_ms,
                                preview_audio_label(preview.microphone),
                                preview_audio_label(preview.system_audio),
                            ));
                            status.set(next.announcement.clone());
                            snapshot.set(Some(next));
                            error.set(None);
                        }
                        (Ok(next), Some(operation_error), []) => {
                            status.set(next.announcement.clone());
                            snapshot.set(Some(next));
                            error.set(Some(operation_error));
                        }
                        _ => {
                            status.set("Native Studio preview rejected.".into());
                            error.set(Some(
                                "Frame rejected malformed, stale, or unrenderable preview media."
                                    .into(),
                            ));
                        }
                    }
                }
                Err(()) => {
                    error.set(Some(
                        "The native command boundary rejected the preview request.".into(),
                    ));
                    status.set("Native Studio preview unavailable.".into());
                }
            }
            busy.set(false);
        });
    }

    fn preview_audio_label(audio: frame_desktop_core::NativeStudioPreviewAudioState) -> String {
        if !audio.source_available {
            "silent (no source)".into()
        } else if audio.muted {
            "muted".into()
        } else {
            format!("active at {} millibels", audio.gain_millibels)
        }
    }

    fn draw_studio_preview(preview: &NativeStudioPreviewOutcome) -> Result<(), ()> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(())?;
        let canvas = document
            .get_element_by_id("studio-preview-canvas")
            .ok_or(())?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| ())?;
        canvas.set_width(preview.width);
        canvas.set_height(preview.height);
        let context = canvas
            .get_context("2d")
            .map_err(|_| ())?
            .ok_or(())?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| ())?;
        let pixel_count = usize::try_from(preview.width)
            .ok()
            .and_then(|width| {
                usize::try_from(preview.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(())?;
        let mut rgba = Vec::with_capacity(pixel_count.checked_mul(4).ok_or(())?);
        for rgb in preview.rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], u8::MAX]);
        }
        if rgba.len() != pixel_count.checked_mul(4).ok_or(())? {
            return Err(());
        }
        let image = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(&rgba),
            preview.width,
            preview.height,
        )
        .map_err(|_| ())?;
        context.put_image_data(&image, 0.0, 0.0).map_err(|_| ())
    }

    fn validated_snapshot(
        previous_input: Option<(AudioMeterSnapshot, CameraPreviewState)>,
        recorder_poll: bool,
        dispatch: &DesktopDispatch,
    ) -> Result<DesktopRuntimeSnapshot, ()> {
        let mut next = dispatch.snapshot.clone();
        let mut previous_event_sequence = None;
        let mut telemetry_count = 0_u8;
        if recorder_poll
            && next.recorder == RecorderState::Recording
            && let Some((meter, camera_preview)) = previous_input
        {
            next.meter = meter;
            next.camera_preview = camera_preview;
        }
        for envelope in &dispatch.events {
            if envelope.protocol_version != DESKTOP_RUNTIME_VERSION
                || previous_event_sequence
                    .is_some_and(|previous| envelope.event_sequence <= previous)
            {
                return Err(());
            }
            previous_event_sequence = Some(envelope.event_sequence);
            if let DesktopRuntimeEvent::InputTelemetry(telemetry) = &envelope.event {
                telemetry_count = telemetry_count.checked_add(1).ok_or(())?;
                if telemetry_count > 1
                    || envelope.owner != WindowRole::Recorder
                    || !telemetry.validate()
                {
                    return Err(());
                }
                next.meter = telemetry.meter;
                next.camera_preview = telemetry.camera_preview;
            }
        }
        Ok(next)
    }

    fn retry_instant_finalize(
        client: RwSignal<Option<DesktopClient>>,
        snapshot: RwSignal<Option<DesktopRuntimeSnapshot>>,
        status: RwSignal<String>,
        error: RwSignal<Option<String>>,
        busy: RwSignal<bool>,
    ) {
        let Some(client) = client.get_untracked() else {
            error.set(Some("The native backend is unavailable.".into()));
            return;
        };
        let Some(handle) = snapshot
            .get_untracked()
            .and_then(|state| state.instant_finalize_handle)
        else {
            error.set(Some("Instant sharing is not configured.".into()));
            return;
        };
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match client.finalize_instant(handle).await {
                Ok(update) => {
                    snapshot.update(|current| {
                        if let Some(state) = current {
                            state.operation_revision = update.operation_revision;
                            state.instant_progress = Some(update.progress);
                            if matches!(
                                update.progress.phase,
                                InstantUiPhaseV1::ShareReady
                                    | InstantUiPhaseV1::Cancelled
                                    | InstantUiPhaseV1::RecoveryRequired
                            ) {
                                state.instant_finalize_handle = None;
                                state.instant_finalize_next_sequence = None;
                            } else {
                                state.instant_finalize_next_sequence =
                                    update.command_sequence.checked_add(1);
                            }
                            state.announcement =
                                instant_progress_announcement(update.progress).into();
                        }
                    });
                    status.set(instant_progress_announcement(update.progress).into());
                    error.set(
                        update
                            .progress
                            .error
                            .map(|code| instant_error_message(code).into()),
                    );
                }
                Err(()) => {
                    error.set(Some(
                        "The native Instant command was rejected. Refresh before retrying.".into(),
                    ));
                    status.set("Instant sharing status was not changed.".into());
                }
            }
            busy.set(false);
        });
    }

    fn public_error(code: PublicErrorCode) -> &'static str {
        match code {
            PublicErrorCode::InvalidRequest => {
                "The operation is not valid in the current backend state."
            }
            PublicErrorCode::Forbidden => "This window does not own that operation.",
            PublicErrorCode::Conflict => "Backend state changed. Refresh and retry.",
            PublicErrorCode::Busy => "Another operation is still running.",
            PublicErrorCode::Unavailable => "The required native adapter is unavailable.",
            PublicErrorCode::Cancelled => "The operation was cancelled.",
            PublicErrorCode::Internal => "The native operation could not be completed.",
        }
    }

    fn recorder_status(snapshot: Option<DesktopRuntimeSnapshot>) -> &'static str {
        match snapshot.map(|snapshot| snapshot.recorder) {
            Some(RecorderState::Idle) => "Idle",
            Some(RecorderState::Preparing) => "Preparing",
            Some(RecorderState::Recording) => "Recording",
            Some(RecorderState::Paused) => "Paused",
            Some(RecorderState::Recoverable) => "Recovery available",
            Some(RecorderState::Ready) => "Project ready",
            Some(RecorderState::Failed { .. }) => "Recording failed",
            None => "Connecting",
        }
    }

    const fn capture_target_kind_label(kind: CaptureTargetKind) -> &'static str {
        match kind {
            CaptureTargetKind::Display => "Display",
            CaptureTargetKind::Window => "Window",
            CaptureTargetKind::Region => "Region",
        }
    }

    fn native_target_pressed(
        state: &DesktopRuntimeSnapshot,
        kind: CaptureTargetKind,
    ) -> Option<bool> {
        let matching_targets = state
            .capture_targets
            .targets
            .iter()
            .filter(|target| target.kind == kind)
            .count();
        (matching_targets == 1).then_some(state.selected_sources.target == Some(kind))
    }

    fn permission_guidance(snapshot: Option<DesktopRuntimeSnapshot>) -> &'static str {
        match snapshot.map(|state| (state.adapter, state.permission)) {
            Some((
                DesktopAdapterKind::NativeMacOs,
                frame_desktop_core::PermissionState::Granted,
            )) => {
                "macOS reports Screen & System Audio Recording access. If access was just granted, quit and reopen Frame before recording."
            }
            Some((
                DesktopAdapterKind::NativeMacOs,
                frame_desktop_core::PermissionState::Denied,
            )) => {
                "Allow Frame in System Settings under Privacy & Security, Screen & System Audio Recording, then quit and reopen Frame."
            }
            Some((DesktopAdapterKind::NativeMacOs, _)) => {
                "macOS Screen & System Audio Recording access has not been confirmed. Recording stays disabled."
            }
            Some((
                DesktopAdapterKind::NativeWindows,
                frame_desktop_core::PermissionState::Granted,
            )) => {
                "Windows Graphics Capture is available. Frame windows remain excluded from recordings."
            }
            Some((
                DesktopAdapterKind::NativeWindows,
                frame_desktop_core::PermissionState::Denied,
            )) => {
                "Windows Graphics Capture is unavailable. Update Windows or review system capture policy, then reopen Frame."
            }
            Some((DesktopAdapterKind::NativeWindows, _)) => {
                "Windows Graphics Capture availability has not been confirmed. Recording stays disabled."
            }
            Some((_, frame_desktop_core::PermissionState::Granted)) => {
                "Screen and device permissions are confirmed."
            }
            Some((_, frame_desktop_core::PermissionState::Denied)) => {
                "Permission was denied. Open system privacy settings and return to Frame."
            }
            _ => "Permission has not been confirmed. Recording stays disabled.",
        }
    }

    fn progress(export: ExportState) -> u16 {
        match export {
            ExportState::Running {
                progress_basis_points,
                ..
            } => progress_basis_points,
            ExportState::Completed { .. } => 10_000,
            _ => 0,
        }
    }

    fn upload_progress(upload: UploadState) -> u32 {
        match upload {
            UploadState::Uploading {
                verified_parts,
                total_parts,
            }
            | UploadState::Paused {
                verified_parts,
                total_parts,
                ..
            } if total_parts > 0 => verified_parts.saturating_mul(100) / total_parts,
            UploadState::Finalizing | UploadState::Completed => 100,
            _ => 0,
        }
    }

    fn instant_phase_label(progress: Option<InstantUiProgressV1>) -> &'static str {
        match progress.map(|progress| progress.phase) {
            Some(InstantUiPhaseV1::Recording) => "Recording locally",
            Some(InstantUiPhaseV1::LocallyRecoverable) => "Safe on this device",
            Some(InstantUiPhaseV1::Uploading) => "Uploading",
            Some(InstantUiPhaseV1::Finalizing) => "Finalizing",
            Some(InstantUiPhaseV1::ShareReady) => "Ready to share",
            Some(InstantUiPhaseV1::Cancelled) => "Cancelled",
            Some(InstantUiPhaseV1::RecoveryRequired) => "Recovery required",
            None => "Unavailable",
        }
    }

    fn show_instant_progress(progress: Option<InstantUiProgressV1>) -> bool {
        progress.is_some_and(|progress| {
            matches!(
                progress.phase,
                InstantUiPhaseV1::Recording
                    | InstantUiPhaseV1::Uploading
                    | InstantUiPhaseV1::Finalizing
                    | InstantUiPhaseV1::ShareReady
            )
        })
    }

    #[component]
    fn MainApp() -> impl IntoView {
        let client = RwSignal::new(None::<DesktopClient>);
        let bootstrap = RwSignal::new(None::<DesktopBootstrap>);
        let snapshot = RwSignal::new(None::<DesktopRuntimeSnapshot>);
        let status = RwSignal::new("Connecting to the native backend…".to_owned());
        let error = RwSignal::new(None::<String>);
        let busy = RwSignal::new(false);
        let selection_start = RwSignal::new(1_000_u64);
        let selection_end = RwSignal::new(80_000_u64);
        let preview_position = RwSignal::new(1_000_u64);
        let preview_summary = RwSignal::new("No Studio preview has been rendered yet.".to_owned());

        Effect::new(move |_| {
            spawn_local(async move {
                match bootstrap_native().await {
                    Ok((_shell, desktop)) => {
                        status.set(desktop.snapshot.announcement.clone());
                        snapshot.set(Some(desktop.snapshot.clone()));
                        client.set(Some(DesktopClient::new(
                            desktop.contexts.clone(),
                            desktop.snapshot.instant_finalize_next_sequence,
                        )));
                        bootstrap.set(Some(desktop));
                    }
                    Err(()) => {
                        status.set(
                            "Native backend unavailable. Privileged controls remain disabled."
                                .into(),
                        );
                        error.set(Some(
                            "Frame could not establish the versioned native command boundary."
                                .into(),
                        ));
                    }
                }
            });
        });

        Effect::new(move |_| {
            if let Ok(handle) = set_interval_with_handle(
                move || {
                    let should_poll = snapshot.get_untracked().is_some_and(|state| {
                        matches!(
                            state.adapter,
                            DesktopAdapterKind::NativeMacOs | DesktopAdapterKind::NativeWindows
                        ) && state.recorder == RecorderState::Recording
                    });
                    if should_poll && !busy.get_untracked() {
                        submit(
                            client,
                            snapshot,
                            status,
                            error,
                            busy,
                            WindowRole::Recorder,
                            IpcCommand::RecorderPoll,
                        );
                    }
                },
                RECORDER_POLL_INTERVAL,
            ) {
                on_cleanup(move || handle.clear());
            }
        });

        Effect::new(move |_| {
            if let Ok(handle) = set_interval_with_handle(
                move || {
                    let should_poll = snapshot.get_untracked().is_some_and(|state| {
                        state.adapter == DesktopAdapterKind::NativeMacOs
                            && matches!(state.export, ExportState::Running { .. })
                    });
                    if should_poll && !busy.get_untracked() {
                        submit(
                            client,
                            snapshot,
                            status,
                            error,
                            busy,
                            WindowRole::Editor,
                            IpcCommand::ExportPoll,
                        );
                    }
                },
                RECORDER_POLL_INTERVAL,
            ) {
                on_cleanup(move || handle.clear());
            }
        });

        let is_fake = move || {
            snapshot
                .get()
                .is_some_and(|state| state.adapter == DesktopAdapterKind::DeterministicFake)
        };
        let is_native = move || {
            snapshot.get().is_some_and(|state| {
                matches!(
                    state.adapter,
                    DesktopAdapterKind::NativeMacOs | DesktopAdapterKind::NativeWindows
                )
            })
        };
        let is_macos_native = move || {
            snapshot
                .get()
                .is_some_and(|state| state.adapter == DesktopAdapterKind::NativeMacOs)
        };
        let supports_capture_targets = move || is_fake() || is_native();
        let can_start = move || {
            snapshot.get().is_some_and(|state| {
                matches!(
                    state.adapter,
                    DesktopAdapterKind::DeterministicFake
                        | DesktopAdapterKind::NativeMacOs
                        | DesktopAdapterKind::NativeWindows
                ) && state.permission == frame_desktop_core::PermissionState::Granted
                    && state.selected_sources.target.is_some()
                    && matches!(
                        state.recorder,
                        RecorderState::Idle | RecorderState::Ready | RecorderState::Failed { .. }
                    )
            }) && !busy.get()
        };
        let can_pause = move || {
            snapshot.get().is_some_and(|state| {
                (state.adapter == DesktopAdapterKind::DeterministicFake
                    || (state.adapter == DesktopAdapterKind::NativeMacOs
                        && (state.settings.microphone_enabled
                            || state.settings.system_audio_enabled
                            || state.settings.camera_enabled)))
                    && state.recorder == RecorderState::Recording
            }) && !busy.get()
        };
        let can_resume = move || {
            snapshot.get().is_some_and(|state| {
                matches!(
                    state.adapter,
                    DesktopAdapterKind::DeterministicFake | DesktopAdapterKind::NativeMacOs
                ) && state.recorder == RecorderState::Paused
            }) && !busy.get()
        };
        let can_stop = move || {
            snapshot.get().is_some_and(|state| {
                (state.adapter == DesktopAdapterKind::DeterministicFake
                    && matches!(
                        state.recorder,
                        RecorderState::Recording | RecorderState::Paused
                    ))
                    || (matches!(
                        state.adapter,
                        DesktopAdapterKind::NativeMacOs | DesktopAdapterKind::NativeWindows
                    ) && matches!(
                        state.recorder,
                        RecorderState::Recording | RecorderState::Paused
                    ))
            }) && !busy.get()
        };
        let can_configure_native_audio = move || {
            snapshot.get().is_some_and(|state| {
                state.adapter == DesktopAdapterKind::NativeMacOs
                    && matches!(
                        state.recorder,
                        RecorderState::Idle | RecorderState::Ready | RecorderState::Failed { .. }
                    )
            }) && !busy.get()
        };
        let fake_paths = move || {
            bootstrap
                .get()
                .and_then(|bootstrap| bootstrap.fake_journey_paths)
        };
        let ready_project = move || {
            snapshot.get().and_then(|state| {
                state
                    .studio_projects
                    .projects
                    .iter()
                    .find(|project| project.status == NativeStudioProjectStatus::Ready)
                    .map(|project| {
                        (
                            state.studio_projects.generation,
                            project.project_token.clone(),
                        )
                    })
            })
        };
        let recovery_project = move || {
            snapshot.get().and_then(|state| {
                state
                    .studio_projects
                    .projects
                    .iter()
                    .find(|project| project.status == NativeStudioProjectStatus::RecoveryRequired)
                    .or_else(|| {
                        state.studio_projects.projects.iter().find(|project| {
                            project.status == NativeStudioProjectStatus::AttentionRequired
                        })
                    })
                    .map(|project| {
                        (
                            state.studio_projects.generation,
                            project.project_token.clone(),
                        )
                    })
            })
        };
        let recovery_selection = move || {
            if is_native() {
                recovery_project()
            } else {
                ready_project()
            }
        };

        view! {
            <UiStyles/>
            <div data-frame-surface="desktop" class="mx-auto max-w-7xl p-4 md:p-8">
            <a class="skip-link" href="#main-content">"Skip to desktop controls"</a>
            <header class="app-header">
                <div>
                    <p class="eyebrow">"Frame desktop"</p>
                    <h1>"Record, recover, edit, and share"</h1>
                    <p>"Every success state below comes from the native Rust backend."</p>
                </div>
                <output class="connection-pill" aria-label="Native connection status">
                    <Badge variant=BadgeVariant::Outline class="connection-pill">
                        {move || if snapshot.get().is_some() { "Backend connected" } else { "Connecting" }}
                    </Badge>
                </output>
            </header>

            <NavigationMenu attr:aria-label="Desktop workspace">
                <a href="#recorder">"Recorder"</a>
                <a href="#recovery">"Recovery"</a>
                <a href="#editor">"Editor"</a>
                <a href="#settings">"Settings"</a>
            </NavigationMenu>

            <main id="main-content" tabindex="-1">
                <Card attr:id="recorder" attr:aria-labelledby="recorder-heading">
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Capture"</p>
                            <h2 id="recorder-heading">"Recorder"</h2>
                        </div>
                        <strong><Badge variant=BadgeVariant::Outline class="state-badge">{move || recorder_status(snapshot.get())}</Badge></strong>
                    </div>

                    <FieldGroup>
                        <legend>"Recording mode"</legend>
                        <ToggleGroup class="button-row" attr:role="group" attr:aria-label="Recording mode">
                            <Button variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.recorder_configuration.mode == RecorderMode::Instant)
                                attr:disabled=move || (!is_fake() && !is_macos_native()) || busy.get()
                                on:click=move |_| {
                                    if let Some(state) = snapshot.get_untracked() {
                                        let (role, command) = if state.adapter == DesktopAdapterKind::NativeMacOs {
                                            (
                                                WindowRole::Settings,
                                                IpcCommand::SettingsApply {
                                                    expected_revision: state.settings.revision,
                                                    mode: RecorderMode::Instant,
                                                    frame_rate: state.settings.frame_rate,
                                                    microphone_enabled: state.settings.microphone_enabled,
                                                    system_audio_enabled: state.settings.system_audio_enabled,
                                                    camera_enabled: false,
                                                    reduced_motion: state.settings.reduced_motion,
                                                },
                                            )
                                        } else {
                                            (
                                                WindowRole::Recorder,
                                                IpcCommand::RecorderConfigure {
                                                    mode: RecorderMode::Instant,
                                                    countdown_seconds: 3,
                                                    exclude_frame_windows: true,
                                                },
                                            )
                                        };
                                        submit(client, snapshot, status, error, busy, role, command);
                                    }
                                }
                            >"Instant"</Button>
                            <Button variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.recorder_configuration.mode == RecorderMode::Studio)
                                attr:disabled=move || (!is_fake() && !is_macos_native()) || busy.get()
                                on:click=move |_| {
                                    if let Some(state) = snapshot.get_untracked() {
                                        let (role, command) = if state.adapter == DesktopAdapterKind::NativeMacOs {
                                            (
                                                WindowRole::Settings,
                                                IpcCommand::SettingsApply {
                                                    expected_revision: state.settings.revision,
                                                    mode: RecorderMode::Studio,
                                                    frame_rate: state.settings.frame_rate,
                                                    microphone_enabled: state.settings.microphone_enabled,
                                                    system_audio_enabled: state.settings.system_audio_enabled,
                                                    camera_enabled: state.settings.camera_enabled,
                                                    reduced_motion: state.settings.reduced_motion,
                                                },
                                            )
                                        } else {
                                            (
                                                WindowRole::Recorder,
                                                IpcCommand::RecorderConfigure {
                                                    mode: RecorderMode::Studio,
                                                    countdown_seconds: 3,
                                                    exclude_frame_windows: true,
                                                },
                                            )
                                        };
                                        submit(client, snapshot, status, error, busy, role, command);
                                    }
                                }
                            >"Studio"</Button>
                        </ToggleGroup>
                    </FieldGroup>

                    <FieldGroup>
                        <legend>"Capture target"</legend>
                        <p id="target-help">"Frame windows are excluded. Choose one opaque target; application names, window titles, and platform identifiers are not sent to the UI."</p>
                        <ToggleGroup class="button-row" attr:aria-describedby="target-help">
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.selected_sources.target == Some(CaptureTargetKind::Display)) attr:disabled=move || !is_fake() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Recorder,
                                IpcCommand::CaptureTargetSelect { kind: CaptureTargetKind::Display, target_token: "fake-display-1".into() }
                            )>"Entire display"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.selected_sources.target == Some(CaptureTargetKind::Window)) attr:disabled=move || !is_fake() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Recorder,
                                IpcCommand::CaptureTargetSelect { kind: CaptureTargetKind::Window, target_token: "fake-window-1".into() }
                            )>"Application window"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.selected_sources.target == Some(CaptureTargetKind::Region)) attr:disabled=move || !is_fake() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Recorder,
                                IpcCommand::CaptureTargetSelect { kind: CaptureTargetKind::Region, target_token: "fake-region-1".into() }
                            )>"Screen region"</Button>
                        </ToggleGroup>
                        <Show when=move || is_native()>
                            <ToggleGroup class="button-row" attr:aria-label="Native capture targets">
                                <For
                                    each=move || snapshot
                                        .get()
                                        .filter(|state| {
                                            state.capture_targets.schema_version
                                                == CAPTURE_TARGET_CATALOG_VERSION
                                        })
                                        .map(|state| state.capture_targets.targets)
                                        .unwrap_or_default()
                                    key=|target| target.token.clone()
                                    children=move |target| {
                                        let token = target.token.clone();
                                        let kind = target.kind;
                                        let label = format!(
                                            "{} {} — {} by {} pixels, {} degree rotation",
                                            capture_target_kind_label(kind),
                                            target.ordinal,
                                            target.width_pixels,
                                            target.height_pixels,
                                            target.rotation_degrees,
                                        );
                                        let accessible_label = label.clone();
                                        view! {
                                            <Button variant=ButtonVariant::Outline
                                                attr:r#type="button"
                                                attr:aria-label=accessible_label
                                                attr:aria-pressed=move || snapshot
                                                    .get()
                                                    .as_ref()
                                                    .and_then(|state| native_target_pressed(state, kind))
                                                attr:disabled=move || busy.get()
                                                on:click=move |_| submit(
                                                    client,
                                                    snapshot,
                                                    status,
                                                    error,
                                                    busy,
                                                    WindowRole::Recorder,
                                                    IpcCommand::CaptureTargetSelect {
                                                        kind,
                                                        target_token: token.clone(),
                                                    },
                                                )
                                            >{label}</Button>
                                        }
                                    }
                                />
                            </ToggleGroup>
                            <RegionPicker client snapshot status error busy role=WindowRole::Recorder />
                        </Show>
                    </FieldGroup>

                    <CardFrame class="permission-card">
                        <h3>"Permissions and devices"</h3>
                        <p>{move || permission_guidance(snapshot.get())}</p>
                        <ButtonGroup class="button-row">
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !supports_capture_targets() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Recorder,
                                IpcCommand::DeviceEnumerate { class: DeviceClass::Display }
                            )>"Refresh capture targets"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !supports_capture_targets() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Recorder,
                                IpcCommand::RecorderPrepare
                            )>{move || if is_macos_native() { "Check macOS access" } else { "Confirm permissions" }}</Button>
                        </ButtonGroup>
                        <p class="device-summary">{move || match snapshot.get().map(|state| state.devices) {
                            Some(DeviceState::Ready(counts)) => format!(
                                "{} displays, {} microphones, {} system audio sources, {} cameras.",
                                counts.displays, counts.microphones, counts.system_audio_sources, counts.cameras
                            ),
                            Some(DeviceState::PermissionDenied) => "Device access denied.".into(),
                            Some(DeviceState::Unavailable) => "Selected device is unavailable.".into(),
                            _ => "No confirmed device inventory.".into(),
                        }}</p>
                    </CardFrame>

                    <Show when=move || is_fake() || is_native()>
                        <div class="meter-grid" aria-label="Live input meters">
                            <Label attr:r#for="microphone-meter">"Microphone"</Label>
                            <Meter attr:id="microphone-meter" attr:min="0" attr:max="10000" attr:value=move || snapshot.get().map_or(0, |state| state.meter.microphone_basis_points)>"Microphone level"</Meter>
                            <Label attr:r#for="system-meter">"System audio"</Label>
                            <Meter attr:id="system-meter" attr:min="0" attr:max="10000" attr:value=move || snapshot.get().map_or(0, |state| state.meter.system_audio_basis_points)>"System audio level"</Meter>
                            <output aria-live="off">{move || match snapshot.get().map(|state| state.camera_preview) {
                                Some(CameraPreviewState::Active) => "Camera preview active",
                                Some(CameraPreviewState::Unavailable) => {
                                    "Camera preview unavailable"
                                }
                                Some(CameraPreviewState::Disabled) | None => "Camera preview disabled",
                            }}</output>
                        </div>
                    </Show>
                    <Show when=move || is_macos_native()>
                        <ButtonGroup class="button-row" attr:aria-label="Live native input controls">
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::Microphone,
                                        gain_milli: 1_000,
                                        muted: false,
                                        enabled: true,
                                    });
                                }
                            }>"Unmute microphone"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::Microphone,
                                        gain_milli: 500,
                                        muted: false,
                                        enabled: true,
                                    });
                                }
                            }>"Microphone 50%"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::Microphone,
                                        gain_milli: 1_000,
                                        muted: true,
                                        enabled: true,
                                    });
                                }
                            }>"Mute microphone"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::SystemAudio,
                                        gain_milli: 1_000,
                                        muted: false,
                                        enabled: true,
                                    });
                                }
                            }>"Unmute system audio"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::SystemAudio,
                                        gain_milli: 500,
                                        muted: false,
                                        enabled: true,
                                    });
                                }
                            }>"System audio 50%"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::SystemAudio,
                                        gain_milli: 1_000,
                                        muted: true,
                                        enabled: true,
                                    });
                                }
                            }>"Mute system audio"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::Camera,
                                        gain_milli: 1_000,
                                        muted: false,
                                        enabled: true,
                                    });
                                }
                            }>"Include camera track"</Button>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() || busy.get() on:click=move |_| {
                                if let Some(client_value) = client.get_untracked() {
                                    submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderInputSet {
                                        intent_id: client_value.next_intent_id(),
                                        class: DeviceClass::Camera,
                                        gain_milli: 1_000,
                                        muted: false,
                                        enabled: false,
                                    });
                                }
                            }>"Exclude camera track"</Button>
                        </ButtonGroup>
                        <p class="privacy-note">
                            "Native macOS capture records the selected target with any enabled confirmed microphone, system-audio, and camera inputs. Excluding the live camera track stops new camera samples from entering the Studio original, while the session-owned camera remains active until pause or stop. Native export is Editable WebM."
                        </p>
                    </Show>

                    <div class="primary-actions" role="group" aria-label="Recording controls">
                        <Button variant=ButtonVariant::Primary attr:r#type="button" attr:disabled=move || !can_start() on:click=move |_| {
                            if let Some(client_value) = client.get_untracked() {
                                let intent_id = client_value.next_intent_id();
                                submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderStart { intent_id });
                            }
                        }>"Start recording"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_pause() on:click=move |_| {
                            if let Some(client_value) = client.get_untracked() {
                                let intent_id = client_value.next_intent_id();
                                submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderPause { intent_id });
                            }
                        }>"Pause"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_resume() on:click=move |_| {
                            if let Some(client_value) = client.get_untracked() {
                                let intent_id = client_value.next_intent_id();
                                submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderResume { intent_id });
                            }
                        }>"Resume"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !can_stop() on:click=move |_| {
                            if let Some(client_value) = client.get_untracked() {
                                let intent_id = client_value.next_intent_id();
                                submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderStop { intent_id });
                            }
                        }>"Stop"</Button>
                        <Button variant=ButtonVariant::Destructive attr:r#type="button" attr:disabled=move || !can_stop() on:click=move |_| {
                            if let Some(client_value) = client.get_untracked() {
                                let intent_id = client_value.next_intent_id();
                                submit(client, snapshot, status, error, busy, WindowRole::Recorder, IpcCommand::RecorderCancel { intent_id });
                            }
                        }>"Cancel recording"</Button>
                    </div>

                    <Card class="instant-sharing" attr:aria-labelledby="instant-sharing-heading">
                        <div class="section-heading compact">
                            <div>
                                <p class="eyebrow">"Native publication"</p>
                                <h3 id="instant-sharing-heading">"Instant sharing"</h3>
                            </div>
                            <output class="state-badge" aria-label="Instant sharing phase">
                                <Badge variant=BadgeVariant::Outline class="state-badge">
                                    {move || instant_phase_label(snapshot.get().and_then(|state| state.instant_progress))}
                                </Badge>
                            </output>
                        </div>

                        <Show
                            when=move || snapshot.get().and_then(|state| state.instant_progress).is_some()
                            fallback=move || view! {
                                <p class="instant-unavailable" role="status" aria-live="polite">
                                    "Native Instant finalization is not configured in this release. No network request can start."
                                </p>
                            }
                        >
                            <Show when=move || show_instant_progress(
                                snapshot.get().and_then(|state| state.instant_progress)
                            )>
                                <Show
                                    when=move || snapshot
                                        .get()
                                        .and_then(|state| state.instant_progress)
                                        .and_then(|progress| progress.progress_basis_points)
                                        .is_some()
                                    fallback=move || view! {
                                        <Progress
                                            class="instant-progress"
                                            attr:max="10000"
                                            attr:aria-label="Instant sharing progress"
                                        >"In progress"</Progress>
                                    }
                                >
                                    <Progress
                                        class="instant-progress"
                                        attr:max="10000"
                                        attr:value=move || snapshot
                                            .get()
                                            .and_then(|state| state.instant_progress)
                                            .and_then(|progress| progress.progress_basis_points)
                                            .unwrap_or(0)
                                        attr:aria-label="Instant sharing progress"
                                    >
                                        {move || format!(
                                            "{} percent",
                                            snapshot
                                                .get()
                                                .and_then(|state| state.instant_progress)
                                                .and_then(|progress| progress.progress_basis_points)
                                                .unwrap_or(0) / 100
                                        )}
                                    </Progress>
                                </Show>
                            </Show>
                            <p class="instant-message" role="status" aria-live="polite">
                                {move || snapshot
                                    .get()
                                    .and_then(|state| state.instant_progress)
                                    .map_or(
                                        "Instant sharing status is unavailable.",
                                        instant_progress_announcement,
                                    )}
                            </p>
                            <Show when=move || snapshot
                                .get()
                                .and_then(|state| state.instant_progress)
                                .and_then(|progress| progress.error)
                                .is_some()
                            >
                                <p class="instant-error" role="alert">
                                    {move || snapshot
                                        .get()
                                        .and_then(|state| state.instant_progress)
                                        .and_then(|progress| progress.error)
                                        .map_or("Instant sharing needs attention.", instant_error_message)}
                                </p>
                            </Show>
                        </Show>

                        <Button variant=ButtonVariant::Outline
                            attr:r#type="button"
                            attr:disabled=move || !snapshot.get().is_some_and(|state| {
                                state.instant_finalize == InstantFinalizeCapabilityState::Available
                                    && state.instant_finalize_handle.is_some()
                                    && state.instant_finalize_next_sequence.is_some()
                                    && state.instant_progress.is_some_and(|progress| progress.retrying)
                            }) || busy.get()
                            on:click=move |_| retry_instant_finalize(
                                client,
                                snapshot,
                                status,
                                error,
                                busy,
                            )
                        >"Retry sharing"</Button>
                        <p class="privacy-note">
                            "The WebView receives only coarse progress, stable error codes, and an opaque native handle. Credentials and recording identities stay in Rust."
                        </p>
                    </Card>
                    <p class="shortcut-help">"Keyboard: Control+Shift+R starts or stops; Control+Shift+P pauses or resumes. Global registration is backend-owned."</p>
                </Card>

                <Card attr:id="recovery" attr:aria-labelledby="recovery-heading">
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Crash-safe"</p>
                            <h2 id="recovery-heading">"Recovery"</h2>
                        </div>
                    </div>
                    <p>"Recovery verifies the durable journal and immutable originals before creating or opening a project. Empty interrupted attempts can be archived explicitly; captured media is never silently deleted."</p>
                    <ButtonGroup class="button-row">
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || (!is_fake() && !is_native()) || busy.get() on:click=move |_| submit(
                            client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryScan
                        )>"Scan for recovery"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || recovery_selection().is_none() || busy.get() on:click=move |_| {
                            if let Some((catalog_generation, project_token)) = recovery_selection() {
                                submit(client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryInspect { catalog_generation, project_token });
                            }
                        }>"Inspect recovery"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || recovery_selection().is_none() || busy.get() on:click=move |_| {
                            if let Some((catalog_generation, project_token)) = recovery_selection() {
                                submit(client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryOpen { catalog_generation, project_token });
                            }
                        }>{move || if is_native() { "Recover durable project" } else { "Open sample recovery" }}</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || recovery_selection().is_none() || busy.get() on:click=move |_| {
                            if let Some((catalog_generation, project_token)) = recovery_selection() {
                                submit(client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryDiscard { catalog_generation, project_token });
                            }
                        }>{move || if is_native() { "Archive empty attempt" } else { "Discard sample recovery" }}</Button>
                    </ButtonGroup>
                </Card>

                <Card attr:id="editor" attr:aria-labelledby="editor-heading">
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Revision fenced"</p>
                            <h2 id="editor-heading">"Editor and timeline"</h2>
                        </div>
                    </div>
                    <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || ready_project().is_none() || busy.get() on:click=move |_| {
                        if let Some((catalog_generation, project_token)) = ready_project() {
                            submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::EditorOpen { catalog_generation, project_token });
                        }
                    }>{move || if is_native() { "Open Studio project" } else { "Open sample project" }}</Button>
                    <FieldGroup class="timeline-controls">
                        <legend>"Numeric timeline alternative"</legend>
                        <p id="timeline-help">"Arrow keys adjust each native range control. The numeric fields expose the same essential trim operation without drag gestures."</p>
                        <Label attr:r#for="selection-start">"Selection start, milliseconds"</Label>
                        <Input
                            attr:id="selection-start"
                            attr:r#type="number"
                            attr:min="0"
                            attr:max="89999"
                            attr:step="1000"
                            prop:value=move || selection_start.get().to_string()
                            on:input=move |event| {
                                if let Ok(value) = event_target_value(&event).parse::<u64>() {
                                    selection_start.set(value.min(selection_end.get().saturating_sub(1)));
                                }
                            }
                            attr:aria-describedby="timeline-help"
                        />
                        <Label attr:r#for="selection-end">"Selection end, milliseconds"</Label>
                        <Input
                            attr:id="selection-end"
                            attr:r#type="number"
                            attr:min="1"
                            attr:max="90000"
                            attr:step="1000"
                            prop:value=move || selection_end.get().to_string()
                            on:input=move |event| {
                                if let Ok(value) = event_target_value(&event).parse::<u64>() {
                                    selection_end.set(value.max(selection_start.get().saturating_add(1)).min(90_000));
                                }
                            }
                            attr:aria-describedby="timeline-help"
                        />
                        <Label attr:r#for="preview-position">"Preview position, milliseconds"</Label>
                        <Input
                            attr:id="preview-position"
                            attr:r#type="number"
                            attr:min="0"
                            attr:max="89999"
                            attr:step="100"
                            prop:value=move || preview_position.get().to_string()
                            on:input=move |event| {
                                if let Ok(value) = event_target_value(&event).parse::<u64>() {
                                    preview_position.set(value.min(89_999));
                                }
                            }
                            attr:aria-describedby="preview-help"
                        />
                    </FieldGroup>
                    <ButtonGroup class="button-row">
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || {
                            busy.get() || snapshot.get().is_none_or(|state| {
                                !matches!(
                                    state.adapter,
                                    DesktopAdapterKind::DeterministicFake
                                        | DesktopAdapterKind::NativeMacOs
                                ) || !matches!(state.editor, EditorState::Ready { .. })
                            })
                        } on:click=move |_| {
                            if let Some(EditorState::Ready {
                                revision,
                                duration_ms,
                                ..
                            }) = snapshot.get_untracked().map(|state| state.editor)
                            {
                                let position_ms =
                                    preview_position.get_untracked().min(duration_ms.saturating_sub(1));
                                preview_position.set(position_ms);
                                submit_preview(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    preview_summary,
                                    NativeStudioPreviewRequest {
                                        editor_revision: revision,
                                        position_ms,
                                    },
                                );
                            }
                        }>"Render preview frame"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !snapshot.get().is_some_and(|state| matches!(state.editor, EditorState::Ready { .. })) || busy.get() on:click=move |_| {
                            if let Some(EditorState::Ready { revision, .. }) = snapshot.get().map(|state| state.editor) {
                                submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::EditorApply {
                                    base_revision: revision,
                                    mutation: EditorMutation::Trim { start_ms: selection_start.get_untracked(), end_ms: selection_end.get_untracked() },
                                });
                            }
                        }>"Trim to selection"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !snapshot.get().is_some_and(|state| matches!(state.editor, EditorState::Ready { dirty: true, .. })) || busy.get() on:click=move |_| {
                            if let Some(EditorState::Ready { revision, .. }) = snapshot.get().map(|state| state.editor) {
                                submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::EditorSave { expected_revision: revision });
                            }
                        }>"Save project"</Button>
                    </ButtonGroup>
                    <div id="preview-help">
                        <canvas
                            id="studio-preview-canvas"
                            width="320"
                            height="180"
                            role="img"
                            aria-label="Decoded Studio preview frame"
                            style="display:block;max-width:100%;height:auto;background:#111827;border-radius:0.5rem;"
                        ></canvas>
                        <p aria-live="polite">{move || preview_summary.get()}</p>
                    </div>

                    <div class="split-grid">
                        <Card attr:aria-labelledby="export-heading">
                            <h3 id="export-heading">"Export"</h3>
                            <Progress attr:max="10000" attr:value=move || snapshot.get().map_or(0, |state| progress(state.export))>
                                {move || format!("{} percent", snapshot.get().map_or(0, |state| progress(state.export) / 100))}
                            </Progress>
                            <ButtonGroup class="button-row">
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || {
                                    if busy.get() {
                                        return true;
                                    }
                                    snapshot.get().is_none_or(|state| match state.adapter {
                                        DesktopAdapterKind::DeterministicFake => {
                                            fake_paths().is_none()
                                                || !matches!(state.editor, EditorState::Ready { dirty: false, .. })
                                        }
                                        DesktopAdapterKind::NativeMacOs => {
                                            let studio_ready = matches!(
                                                state.editor,
                                                EditorState::Ready { dirty: false, .. }
                                            ) && state.studio_export_destination.is_some();
                                            let capture_ready = state
                                                .capture_artifact
                                                .as_ref()
                                                .filter(|artifact| {
                                                    artifact.schema_version
                                                        == CAPTURE_ARTIFACT_SUMMARY_VERSION
                                                })
                                                .and_then(|artifact| {
                                                    artifact.editable_webm_output_path.as_ref()
                                                })
                                                .is_some();
                                            !studio_ready && !capture_ready
                                        }
                                        DesktopAdapterKind::NativeWindows => state
                                            .capture_artifact
                                            .as_ref()
                                            .filter(|artifact| {
                                                artifact.schema_version
                                                    == CAPTURE_ARTIFACT_SUMMARY_VERSION
                                            })
                                            .and_then(|artifact| artifact.editable_webm_output_path.as_ref())
                                            .is_none(),
                                        DesktopAdapterKind::Unavailable => true,
                                    })
                                } on:click=move |_| {
                                    let Some(state) = snapshot.get_untracked() else {
                                        return;
                                    };
                                    match state.adapter {
                                        DesktopAdapterKind::DeterministicFake => {
                                            if let (Some(paths), EditorState::Ready { project_revision, .. }) = (fake_paths(), state.editor) {
                                                submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::ExportStart {
                                                    project_revision,
                                                    output_path: paths.export,
                                                    profile: ExportProfile::DistributionMp4,
                                                });
                                            }
                                        }
                                        DesktopAdapterKind::NativeMacOs => {
                                            if let (
                                                EditorState::Ready {
                                                    project_revision,
                                                    dirty: false,
                                                    ..
                                                },
                                                Some(destination),
                                            ) = (
                                                state.editor,
                                                state.studio_export_destination,
                                            ) {
                                                submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::ExportStart {
                                                    project_revision,
                                                    output_path: destination.output_path,
                                                    profile: destination.profile,
                                                });
                                            } else if let Some(artifact) = state.capture_artifact
                                                && artifact.schema_version
                                                    == CAPTURE_ARTIFACT_SUMMARY_VERSION
                                                && let Some(output_path) = artifact.editable_webm_output_path
                                            {
                                                submit(client, snapshot, status, error, busy, WindowRole::Export, IpcCommand::ExportStart {
                                                    project_revision: artifact.artifact_revision,
                                                    output_path,
                                                    profile: ExportProfile::EditableWebm,
                                                });
                                            }
                                        }
                                        DesktopAdapterKind::NativeWindows => {
                                            if let Some(artifact) = state.capture_artifact
                                                && artifact.schema_version
                                                    == CAPTURE_ARTIFACT_SUMMARY_VERSION
                                                && let Some(output_path) = artifact.editable_webm_output_path
                                            {
                                                submit(client, snapshot, status, error, busy, WindowRole::Export, IpcCommand::ExportStart {
                                                    project_revision: artifact.artifact_revision,
                                                    output_path,
                                                    profile: ExportProfile::EditableWebm,
                                                });
                                            }
                                        }
                                        DesktopAdapterKind::Unavailable => {}
                                    }
                                }>{move || {
                                    if snapshot.get().is_some_and(|state| {
                                        state.adapter == DesktopAdapterKind::NativeMacOs
                                            && state.studio_export_destination.is_some()
                                            && matches!(
                                                state.editor,
                                                EditorState::Ready { dirty: false, .. }
                                            )
                                    }) {
                                        "Export distribution MP4"
                                    } else if is_native() {
                                        "Export editable WebM"
                                    } else {
                                        "Start export"
                                    }
                                }}</Button>
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !snapshot.get().is_some_and(|state| matches!(state.adapter, DesktopAdapterKind::DeterministicFake | DesktopAdapterKind::NativeMacOs) && matches!(state.export, ExportState::Running { .. })) || busy.get() on:click=move |_| {
                                    if let Some(client_value) = client.get_untracked() {
                                        let intent_id = client_value.next_intent_id();
                                        submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::ExportCancel { intent_id });
                                    }
                                }>"Cancel export"</Button>
                            </ButtonGroup>
                        </Card>
                        <Card attr:aria-labelledby="upload-heading">
                            <h3 id="upload-heading">"Upload"</h3>
                            <Progress attr:max="100" attr:value=move || snapshot.get().map_or(0, |state| upload_progress(state.upload))>
                                {move || format!("{} percent", snapshot.get().map_or(0, |state| upload_progress(state.upload)))}
                            </Progress>
                            <ButtonGroup class="button-row">
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || fake_paths().is_none() || busy.get() on:click=move |_| {
                                    if let (Some(paths), Some(client_value)) = (fake_paths(), client.get_untracked()) {
                                        let upload_intent = client_value.next_intent_id();
                                        submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::UploadStart { source_path: paths.media, upload_intent });
                                    }
                                }>"Start upload"</Button>
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !snapshot.get().is_some_and(|state| matches!(state.upload, UploadState::Uploading { .. })) || busy.get() on:click=move |_| {
                                    if let Some(client_value) = client.get_untracked() {
                                        let intent_id = client_value.next_intent_id();
                                        submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::UploadPause { intent_id });
                                    }
                                }>"Pause upload"</Button>
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !snapshot.get().is_some_and(|state| matches!(state.upload, UploadState::Paused { .. })) || busy.get() on:click=move |_| {
                                    if let Some(client_value) = client.get_untracked() {
                                        let intent_id = client_value.next_intent_id();
                                        submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::UploadResume { intent_id });
                                    }
                                }>"Resume upload"</Button>
                            </ButtonGroup>
                        </Card>
                    </div>
                </Card>

                <Card attr:id="settings" attr:aria-labelledby="settings-heading">
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Preferences"</p>
                            <h2 id="settings-heading">"Settings, presets, and updates"</h2>
                        </div>
                    </div>
                    <p>{move || snapshot.get().map_or_else(
                        || "Settings are loading.".into(),
                        |state| format!("Settings revision {}. {} frames per second.", state.settings.revision, state.settings.frame_rate),
                    )}</p>
                    <Show when=move || is_macos_native()>
                        <div class="privacy-note" aria-labelledby="native-audio-heading">
                            <h3 id="native-audio-heading">"Native macOS inputs"</h3>
                            <p id="native-audio-help">
                                "Microphone, system audio, and camera are optional. Frame uses the confirmed macOS defaults, excludes its own process audio, and stores camera as an isolated Studio original."
                            </p>
                            <ButtonGroup class="button-row">
                                <Button variant=ButtonVariant::Outline
                                    attr:r#type="button"
                                    attr:aria-describedby="native-audio-help"
                                    attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.settings.microphone_enabled)
                                    attr:disabled=move || !can_configure_native_audio()
                                    on:click=move |_| {
                                        if let Some(state) = snapshot.get_untracked() {
                                            submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::SettingsApply {
                                                expected_revision: state.settings.revision,
                                                mode: state.settings.mode,
                                                frame_rate: state.settings.frame_rate,
                                                microphone_enabled: !state.settings.microphone_enabled,
                                                system_audio_enabled: state.settings.system_audio_enabled,
                                                camera_enabled: state.settings.camera_enabled,
                                                reduced_motion: state.settings.reduced_motion,
                                            });
                                        }
                                    }
                                >{move || if snapshot.get().is_some_and(|state| state.settings.microphone_enabled) {
                                    "Microphone: on"
                                } else {
                                    "Microphone: off"
                                }}</Button>
                                <Button variant=ButtonVariant::Outline
                                    attr:r#type="button"
                                    attr:aria-describedby="native-audio-help"
                                    attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.settings.system_audio_enabled)
                                    attr:disabled=move || !can_configure_native_audio()
                                    on:click=move |_| {
                                        if let Some(state) = snapshot.get_untracked() {
                                            submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::SettingsApply {
                                                expected_revision: state.settings.revision,
                                                mode: state.settings.mode,
                                                frame_rate: state.settings.frame_rate,
                                                microphone_enabled: state.settings.microphone_enabled,
                                                system_audio_enabled: !state.settings.system_audio_enabled,
                                                camera_enabled: state.settings.camera_enabled,
                                                reduced_motion: state.settings.reduced_motion,
                                            });
                                        }
                                    }
                                >{move || if snapshot.get().is_some_and(|state| state.settings.system_audio_enabled) {
                                    "System audio: on"
                                } else {
                                    "System audio: off"
                                }}</Button>
                                <Button variant=ButtonVariant::Outline
                                    attr:r#type="button"
                                    attr:aria-describedby="native-audio-help"
                                    attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.settings.camera_enabled)
                                    attr:disabled=move || !can_configure_native_audio() || snapshot.get().is_some_and(|state| state.settings.mode != RecorderMode::Studio && !state.settings.camera_enabled)
                                    on:click=move |_| {
                                        if let Some(state) = snapshot.get_untracked() {
                                            submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::SettingsApply {
                                                expected_revision: state.settings.revision,
                                                mode: state.settings.mode,
                                                frame_rate: state.settings.frame_rate,
                                                microphone_enabled: state.settings.microphone_enabled,
                                                system_audio_enabled: state.settings.system_audio_enabled,
                                                camera_enabled: !state.settings.camera_enabled,
                                                reduced_motion: state.settings.reduced_motion,
                                            });
                                        }
                                    }
                                >{move || if snapshot.get().is_some_and(|state| state.settings.camera_enabled) {
                                    "Camera: on"
                                } else {
                                    "Camera: off"
                                }}</Button>
                            </ButtonGroup>
                        </div>
                    </Show>
                    <ButtonGroup class="button-row">
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || snapshot.get().is_none() || busy.get() on:click=move |_| {
                            if let Some(state) = snapshot.get_untracked() {
                                submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::PresetApply {
                                    preset_token: "preset-balanced".into(),
                                    expected_settings_revision: state.settings.revision,
                                });
                            }
                        }>"Apply balanced preset"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || snapshot.get().is_none() || busy.get() on:click=move |_| {
                            if let Some(state) = snapshot.get_untracked() {
                                submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::PresetApply {
                                    preset_token: "preset-quality".into(),
                                    expected_settings_revision: state.settings.revision,
                                });
                            }
                        }>"Apply quality preset"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || snapshot.get().is_none() || busy.get() on:click=move |_| {
                            if let Some(state) = snapshot.get_untracked() {
                                submit(client, snapshot, status, error, busy, WindowRole::Settings, IpcCommand::SettingsApply {
                                    expected_revision: state.settings.revision,
                                    mode: state.settings.mode,
                                    frame_rate: state.settings.frame_rate,
                                    microphone_enabled: state.settings.microphone_enabled,
                                    system_audio_enabled: state.settings.system_audio_enabled,
                                    camera_enabled: state.settings.camera_enabled,
                                    reduced_motion: !state.settings.reduced_motion,
                                });
                            }
                        }>"Toggle reduced motion"</Button>
                    </ButtonGroup>
                    <aside aria-labelledby="legacy-heading">
                        <Alert class="legacy-note">
                            <h3 id="legacy-heading">"Legacy desktop safety"</h3>
                            <p>"Frame scans Cap's default and remembered recording folders without changing them. Import copies verified media into immutable Frame originals; projects with unsupported effects stay in Cap for review. The previous signed desktop remains selectable until parity gate 29 is approved."</p>
                            <Button variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:disabled=move || !is_native() || busy.get()
                                on:click=move |_| submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Main,
                                    IpcCommand::LegacyProjectScan,
                                )
                            >"Scan Cap projects read-only"</Button>
                            <p aria-live="polite">{move || snapshot.get().map_or_else(
                                || "Cap project migration status is loading.".into(),
                                |state| match state.legacy_projects.availability {
                                    LegacyProjectCatalogAvailability::Unavailable => {
                                        "Cap project scanning is available in native macOS and Windows builds; importing is currently macOS-only.".into()
                                    }
                                    LegacyProjectCatalogAvailability::Ready if state.legacy_projects.projects.is_empty() => {
                                        "No Cap projects were found in the known recording folders.".into()
                                    }
                                    LegacyProjectCatalogAvailability::Ready => format!(
                                        "{} Cap projects were inspected. Filesystem paths and project names remain in Rust.",
                                        state.legacy_projects.projects.len()
                                    ),
                                },
                            )}</p>
                            <div class="project-list" aria-label="Cap project compatibility results">
                                <For
                                    each=move || snapshot
                                        .get()
                                        .map(|state| state.legacy_projects.projects)
                                        .unwrap_or_default()
                                    key=|project| project.project_token.clone()
                                    children=move |project| {
                                        let token = project.project_token.clone();
                                        let ordinal = project.ordinal;
                                        let project_status = project.status;
                                        let label = match project_status {
                                            LegacyProjectStatus::Importable => format!(
                                                "Cap project {ordinal}: importable, {} media assets and {} supported edits.",
                                                project.source_asset_count,
                                                project.supported_effect_count,
                                            ),
                                            LegacyProjectStatus::Imported => format!(
                                                "Cap project {ordinal}: already copied into Frame; the Cap source remains unchanged."
                                            ),
                                            LegacyProjectStatus::NeedsReview => format!(
                                                "Cap project {ordinal}: keep in Cap for review; {} unsupported effects were found.",
                                                project.unsupported_effect_count,
                                            ),
                                            LegacyProjectStatus::Unsupported => format!(
                                                "Cap project {ordinal}: created by a newer unsupported Cap format."
                                            ),
                                            LegacyProjectStatus::Invalid => format!(
                                                "Cap project {ordinal}: incomplete or invalid; no files were copied."
                                            ),
                                        };
                                        view! {
                                            <Card>
                                                <p>{label}</p>
                                                <Button variant=ButtonVariant::Outline
                                                    attr:r#type="button"
                                                    attr:disabled=move || !is_macos_native() || project_status != LegacyProjectStatus::Importable || busy.get()
                                                    on:click=move |_| {
                                                        if let Some(state) = snapshot.get_untracked()
                                                            && state.legacy_projects.generation > 0
                                                        {
                                                            submit(
                                                                client,
                                                                snapshot,
                                                                status,
                                                                error,
                                                                busy,
                                                                WindowRole::Main,
                                                                IpcCommand::LegacyProjectImport {
                                                                    catalog_generation: state.legacy_projects.generation,
                                                                    project_token: token.clone(),
                                                                },
                                                            );
                                                        }
                                                    }
                                                >"Copy into Frame"</Button>
                                            </Card>
                                        }
                                    }
                                />
                            </div>
                        </Alert>
                    </aside>
                    <div class="split-grid">
                        <Card attr:aria-labelledby="lifecycle-heading">
                            <h3 id="lifecycle-heading">"Hotkeys, tray, and overlay"</h3>
                            <p>{move || snapshot.get().map_or("Lifecycle unavailable.", |state| {
                                if state.lifecycle.hotkeys_registered { "Global hotkeys registered by backend." } else { "Global hotkeys are not registered." }
                            })}</p>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || busy.get() || snapshot.get().is_some_and(|state| state.lifecycle.hotkeys_registered) on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Main,
                                IpcCommand::Lifecycle { action: LifecycleAction::RegisterHotkeys }
                            )>"Register global hotkeys"</Button>
                        </Card>
                        <Card attr:aria-labelledby="update-heading">
                            <h3 id="update-heading">"Updates"</h3>
                            <p>{move || match snapshot.get().map(|state| state.update) {
                                Some(UpdateState::Unavailable { .. }) => "Signed updates are unavailable in this build.",
                                Some(UpdateState::Current { .. }) => "Frame is current.",
                                Some(UpdateState::Available { .. }) => "An update is available.",
                                Some(UpdateState::PreviousAvailable { .. }) => "The previous signed desktop is available.",
                                Some(UpdateState::ReadyToRelaunch { .. }) => "Update installed; relaunch is ready.",
                                None => "Update status unavailable.",
                            }}</p>
                            <ButtonGroup>
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || snapshot.get().is_none_or(|state| matches!(state.update, UpdateState::Unavailable { .. } | UpdateState::PreviousAvailable { .. })) || busy.get() on:click=move |_| {
                                    if let Some(state) = snapshot.get_untracked() {
                                        let (action, expected_revision) = match state.update {
                                            UpdateState::Unavailable { .. } | UpdateState::PreviousAvailable { .. } => return,
                                            UpdateState::Current { revision } => (UpdateAction::Check, revision),
                                            UpdateState::Available { revision } => (UpdateAction::Install, revision),
                                            UpdateState::ReadyToRelaunch { revision } => (UpdateAction::Relaunch, revision),
                                        };
                                        submit(client, snapshot, status, error, busy, WindowRole::Main, IpcCommand::Update { action, expected_revision });
                                    }
                                }>{move || match snapshot.get().map(|state| state.update) {
                                    Some(UpdateState::Available { .. }) => "Install update",
                                    Some(UpdateState::ReadyToRelaunch { .. }) => "Relaunch Frame",
                                    _ => "Check for updates",
                                }}</Button>
                                <Button variant=ButtonVariant::Ghost attr:r#type="button" attr:disabled=move || snapshot.get().is_none_or(|state| !state.legacy_desktop_selectable || matches!(state.update, UpdateState::Unavailable { .. } | UpdateState::ReadyToRelaunch { .. })) || busy.get() on:click=move |_| {
                                    if let Some(state) = snapshot.get_untracked() {
                                        let (action, expected_revision) = match state.update {
                                            UpdateState::Current { revision } | UpdateState::Available { revision } => (UpdateAction::CheckPrevious, revision),
                                            UpdateState::PreviousAvailable { revision } => (UpdateAction::InstallPrevious, revision),
                                            UpdateState::Unavailable { .. } | UpdateState::ReadyToRelaunch { .. } => return,
                                        };
                                        submit(client, snapshot, status, error, busy, WindowRole::Main, IpcCommand::Update { action, expected_revision });
                                    }
                                }>{move || match snapshot.get().map(|state| state.update) {
                                    Some(UpdateState::PreviousAvailable { .. }) => "Install previous signed desktop",
                                    _ => "Check previous signed desktop",
                                }}</Button>
                            </ButtonGroup>
                        </Card>
                    </div>
                </Card>
            </main>

            <footer>
                <Alert attr:id="backend-status" class="status" attr:role="status" attr:aria-live="polite" attr:aria-atomic="true">
                    {move || status.get()}
                </Alert>
            </footer>

            {move || error.get().map(|message| view! {
                <DialogOverlay>
                    <DialogContent attr:role="alertdialog" attr:aria-modal="true" attr:aria-labelledby="error-title" attr:aria-describedby="error-message">
                        <h2 id="error-title">"Desktop operation needs attention"</h2>
                        <p id="error-message">{message}</p>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:autofocus=true on:click=move |_| error.set(None)>"Dismiss error"</Button>
                    </DialogContent>
                </DialogOverlay>
            })}
            </div>
        }
    }

    #[component]
    fn OverlayApp() -> impl IntoView {
        let client = RwSignal::new(None::<DesktopClient>);
        let snapshot = RwSignal::new(None::<DesktopRuntimeSnapshot>);
        let status = RwSignal::new("Connecting to recording controls…".to_owned());
        let error = RwSignal::new(None::<String>);
        let busy = RwSignal::new(false);

        Effect::new(move |_| {
            spawn_local(async move {
                match bootstrap_native().await {
                    Ok((_shell, desktop)) => {
                        status.set(desktop.snapshot.announcement.clone());
                        snapshot.set(Some(desktop.snapshot.clone()));
                        client.set(Some(DesktopClient::new(
                            desktop.contexts,
                            desktop.snapshot.instant_finalize_next_sequence,
                        )));
                    }
                    Err(()) => {
                        status.set("Native recording controls are unavailable.".into());
                        error.set(Some(
                            "The overlay could not establish its scoped Rust command boundary."
                                .into(),
                        ));
                    }
                }
            });
        });

        view! {
            <UiStyles/>
            <main data-frame-surface="overlay" class="p-4" aria-labelledby="overlay-heading">
                <Card>
                    <h1 id="overlay-heading">"Recording controls"</h1>
                    <p aria-live="polite">{move || recorder_status(snapshot.get())}</p>
                    <ButtonGroup class="button-row" attr:aria-label="Recording controls">
                        <Button
                            variant=ButtonVariant::Outline
                            attr:r#type="button"
                            attr:disabled=move || busy.get() || !snapshot.get().is_some_and(|state| state.recorder == RecorderState::Recording)
                            on:click=move |_| {
                                let Some(client_value) = client.get_untracked() else { return; };
                                submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Overlay,
                                    IpcCommand::RecorderPause {
                                        intent_id: client_value.next_intent_id(),
                                    },
                                );
                            }
                        >"Pause"</Button>
                        <Button
                            variant=ButtonVariant::Outline
                            attr:r#type="button"
                            attr:disabled=move || busy.get() || !snapshot.get().is_some_and(|state| state.recorder == RecorderState::Paused)
                            on:click=move |_| {
                                let Some(client_value) = client.get_untracked() else { return; };
                                submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Overlay,
                                    IpcCommand::RecorderResume {
                                        intent_id: client_value.next_intent_id(),
                                    },
                                );
                            }
                        >"Resume"</Button>
                        <Button
                            variant=ButtonVariant::Primary
                            attr:r#type="button"
                            attr:disabled=move || busy.get() || !snapshot.get().is_some_and(|state| matches!(state.recorder, RecorderState::Recording | RecorderState::Paused))
                            on:click=move |_| {
                                let Some(client_value) = client.get_untracked() else { return; };
                                submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Overlay,
                                    IpcCommand::RecorderStop {
                                        intent_id: client_value.next_intent_id(),
                                    },
                                );
                            }
                        >"Stop"</Button>
                        <Button
                            variant=ButtonVariant::Ghost
                            attr:r#type="button"
                            attr:disabled=move || busy.get()
                            on:click=move |_| submit(
                                client,
                                snapshot,
                                status,
                                error,
                                busy,
                                WindowRole::Overlay,
                                IpcCommand::Lifecycle { action: LifecycleAction::HideOverlay },
                            )
                        >"Hide controls"</Button>
                    </ButtonGroup>
                    <Alert attr:role="status" attr:aria-live="polite">{move || status.get()}</Alert>
                    {move || error.get().map(|message| view! {
                        <Alert attr:role="alert">{message}</Alert>
                    })}
                </Card>
            </main>
        }
    }

    #[component]
    fn TargetPickerApp() -> impl IntoView {
        let client = RwSignal::new(None::<DesktopClient>);
        let snapshot = RwSignal::new(None::<DesktopRuntimeSnapshot>);
        let status = RwSignal::new("Connecting to capture targets…".to_owned());
        let error = RwSignal::new(None::<String>);
        let busy = RwSignal::new(false);

        Effect::new(move |_| {
            spawn_local(async move {
                match bootstrap_native().await {
                    Ok((_shell, desktop)) => {
                        let surface_client = DesktopClient::new(
                            desktop.contexts,
                            desktop.snapshot.instant_finalize_next_sequence,
                        );
                        status.set(desktop.snapshot.announcement.clone());
                        snapshot.set(Some(desktop.snapshot));
                        client.set(Some(surface_client));
                        submit(
                            client,
                            snapshot,
                            status,
                            error,
                            busy,
                            WindowRole::TargetPicker,
                            IpcCommand::DeviceEnumerate {
                                class: DeviceClass::Display,
                            },
                        );
                    }
                    Err(()) => {
                        status.set("Capture targets are unavailable.".into());
                        error.set(Some(
                            "The target picker could not establish its scoped Rust command boundary."
                                .into(),
                        ));
                    }
                }
            });
        });

        view! {
            <UiStyles/>
            <main data-frame-surface="target-picker" class="p-4" aria-labelledby="target-picker-heading">
                <Card>
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Capture source"</p>
                            <h1 id="target-picker-heading">"Choose what Frame records"</h1>
                        </div>
                        <Button
                            variant=ButtonVariant::Ghost
                            attr:r#type="button"
                            attr:disabled=move || busy.get()
                            on:click=move |_| submit(
                                client,
                                snapshot,
                                status,
                                error,
                                busy,
                                WindowRole::TargetPicker,
                                IpcCommand::Lifecycle { action: LifecycleAction::HideTargetPicker },
                            )
                        >"Close picker"</Button>
                    </div>
                    <p id="target-privacy">
                        "Targets are deliberately identified only by type, ordinal, dimensions, and an opaque session token."
                    </p>
                    <ToggleGroup class="button-row" attr:aria-describedby="target-privacy">
                        <For
                            each=move || snapshot
                                .get()
                                .filter(|state| state.capture_targets.schema_version == CAPTURE_TARGET_CATALOG_VERSION)
                                .map(|state| state.capture_targets.targets)
                                .unwrap_or_default()
                            key=|target| target.token.clone()
                            children=move |target| {
                                let token = target.token.clone();
                                let kind = target.kind;
                                let label = format!(
                                    "{} {} — {} by {} pixels",
                                    capture_target_kind_label(kind),
                                    target.ordinal,
                                    target.width_pixels,
                                    target.height_pixels,
                                );
                                view! {
                                    <Button
                                        variant=ButtonVariant::Outline
                                        attr:r#type="button"
                                        attr:aria-pressed=move || snapshot
                                            .get()
                                            .as_ref()
                                            .and_then(|state| native_target_pressed(state, kind))
                                        attr:disabled=move || busy.get()
                                        on:click=move |_| submit(
                                            client,
                                            snapshot,
                                            status,
                                            error,
                                            busy,
                                            WindowRole::TargetPicker,
                                            IpcCommand::CaptureTargetSelect {
                                                kind,
                                                target_token: token.clone(),
                                            },
                                        )
                                    >{label}</Button>
                                }
                            }
                        />
                    </ToggleGroup>
                    <RegionPicker client snapshot status error busy role=WindowRole::TargetPicker />
                    <Button
                        variant=ButtonVariant::Outline
                        attr:r#type="button"
                        attr:disabled=move || busy.get()
                        on:click=move |_| submit(
                            client,
                            snapshot,
                            status,
                            error,
                            busy,
                            WindowRole::TargetPicker,
                            IpcCommand::DeviceEnumerate { class: DeviceClass::Display },
                        )
                    >"Refresh targets"</Button>
                    <Alert attr:role="status" attr:aria-live="polite">{move || status.get()}</Alert>
                    {move || error.get().map(|message| view! {
                        <Alert attr:role="alert">{message}</Alert>
                    })}
                </Card>
            </main>
        }
    }

    fn active_surface() -> &'static str {
        let search = web_sys::window()
            .and_then(|window| window.location().search().ok())
            .unwrap_or_default();
        if search
            .split('&')
            .any(|part| part.trim_start_matches('?') == "frame_surface=overlay")
        {
            "overlay"
        } else if search
            .split('&')
            .any(|part| part.trim_start_matches('?') == "frame_surface=target-picker")
        {
            "target-picker"
        } else {
            "main"
        }
    }

    #[component]
    fn App() -> impl IntoView {
        match active_surface() {
            "overlay" => view! { <OverlayApp/> }.into_any(),
            "target-picker" => view! { <TargetPickerApp/> }.into_any(),
            _ => view! { <MainApp/> }.into_any(),
        }
    }

    pub fn mount() {
        leptos::mount::mount_to_body(App);
    }
}

#[cfg(all(target_arch = "wasm32", feature = "csr"))]
fn main() {
    browser::mount();
}

#[cfg(not(all(target_arch = "wasm32", feature = "csr")))]
fn main() {}
