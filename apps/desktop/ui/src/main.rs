#[cfg(all(target_arch = "wasm32", feature = "csr"))]
mod browser {
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
        CAPTURE_ARTIFACT_SUMMARY_VERSION, CAPTURE_TARGET_CATALOG_VERSION, CaptureTargetKind,
        CommandOutcome, DESKTOP_RUNTIME_VERSION, DesktopAdapterKind, DesktopBootstrap,
        DesktopDispatch, DesktopRuntimeSnapshot, DesktopWindowContext, DeviceClass, DeviceState,
        EditorMutation, EditorState, ExportProfile, ExportState, IPC_PROTOCOL_VERSION,
        InstantFinalizeCapabilityState, InstantFinalizeCommandV1, InstantFinalizeHandle,
        InstantFinalizeUiUpdate, IpcCommand, LifecycleAction, PublicErrorCode,
        RecorderAdapterState, RecorderMode, RecorderState, RequestEnvelope, RequestId,
        ShellCapabilities, UpdateAction, UpdateState, UploadState, WindowRole,
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
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::spawn_local;

    const RECORDER_POLL_INTERVAL: Duration = Duration::from_secs(1);

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
        busy.set(true);
        spawn_local(async move {
            match client.dispatch(role, command).await {
                Ok(dispatch) => {
                    let operation_error = match dispatch.response.outcome {
                        CommandOutcome::Ok { .. } => None,
                        CommandOutcome::Error { code, .. } => Some(public_error(code).into()),
                    };
                    status.set(dispatch.snapshot.announcement.clone());
                    snapshot.set(Some(dispatch.snapshot));
                    error.set(operation_error);
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
    fn App() -> impl IntoView {
        let client = RwSignal::new(None::<DesktopClient>);
        let bootstrap = RwSignal::new(None::<DesktopBootstrap>);
        let snapshot = RwSignal::new(None::<DesktopRuntimeSnapshot>);
        let status = RwSignal::new("Connecting to the native backend…".to_owned());
        let error = RwSignal::new(None::<String>);
        let busy = RwSignal::new(false);
        let selection_start = RwSignal::new(1_000_u64);
        let selection_end = RwSignal::new(80_000_u64);

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
                        state.adapter == DesktopAdapterKind::NativeMacOs
                            && state.recorder == RecorderState::Recording
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

        let is_fake = move || {
            snapshot
                .get()
                .is_some_and(|state| state.adapter == DesktopAdapterKind::DeterministicFake)
        };
        let is_native = move || {
            snapshot
                .get()
                .is_some_and(|state| state.adapter == DesktopAdapterKind::NativeMacOs)
        };
        let supports_capture_targets = move || is_fake() || is_native();
        let can_start = move || {
            snapshot.get().is_some_and(|state| {
                matches!(
                    state.adapter,
                    DesktopAdapterKind::DeterministicFake | DesktopAdapterKind::NativeMacOs
                ) && state.permission == frame_desktop_core::PermissionState::Granted
                    && state.selected_sources.target.is_some()
                    && (state.adapter == DesktopAdapterKind::DeterministicFake
                        || (!state.settings.microphone_enabled && !state.settings.camera_enabled))
                    && matches!(
                        state.recorder,
                        RecorderState::Idle | RecorderState::Ready | RecorderState::Failed { .. }
                    )
            }) && !busy.get()
        };
        let can_pause = move || {
            is_fake()
                && snapshot
                    .get()
                    .is_some_and(|state| state.recorder == RecorderState::Recording)
                && !busy.get()
        };
        let can_resume = move || {
            is_fake()
                && snapshot
                    .get()
                    .is_some_and(|state| state.recorder == RecorderState::Paused)
                && !busy.get()
        };
        let can_stop = move || {
            snapshot.get().is_some_and(|state| {
                (state.adapter == DesktopAdapterKind::DeterministicFake
                    && matches!(
                        state.recorder,
                        RecorderState::Recording | RecorderState::Paused
                    ))
                    || (state.adapter == DesktopAdapterKind::NativeMacOs
                        && state.recorder == RecorderState::Recording)
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
                                attr:disabled=move || !is_fake() || busy.get()
                                on:click=move |_| submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Recorder,
                                    IpcCommand::RecorderConfigure {
                                        mode: RecorderMode::Instant,
                                        countdown_seconds: 3,
                                        exclude_frame_windows: true,
                                    },
                                )
                            >"Instant"</Button>
                            <Button variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:aria-pressed=move || snapshot.get().is_some_and(|state| state.recorder_configuration.mode == RecorderMode::Studio)
                                attr:disabled=move || !is_fake() || busy.get()
                                on:click=move |_| submit(
                                    client,
                                    snapshot,
                                    status,
                                    error,
                                    busy,
                                    WindowRole::Recorder,
                                    IpcCommand::RecorderConfigure {
                                        mode: RecorderMode::Studio,
                                        countdown_seconds: 3,
                                        exclude_frame_windows: true,
                                    },
                                )
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
                            )>{move || if is_native() { "Check macOS access" } else { "Confirm permissions" }}</Button>
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

                    <Show when=move || is_fake()>
                        <div class="meter-grid" aria-label="Live input meters">
                            <Label attr:r#for="microphone-meter">"Microphone"</Label>
                            <Meter attr:id="microphone-meter" attr:min="0" attr:max="10000" attr:value=move || snapshot.get().map_or(0, |state| state.meter.microphone_basis_points)>"Microphone level"</Meter>
                            <Label attr:r#for="system-meter">"System audio"</Label>
                            <Meter attr:id="system-meter" attr:min="0" attr:max="10000" attr:value=move || snapshot.get().map_or(0, |state| state.meter.system_audio_basis_points)>"System audio level"</Meter>
                        </div>
                    </Show>
                    <Show when=move || is_native()>
                        <p class="privacy-note">
                            "Native macOS capture records the selected target and can optionally include system audio. Microphone, camera, pause/resume, and MP4 export remain unavailable; native export is Editable WebM."
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
                    <p>"Recovery opens a preserved copy. Discard is explicit and never mutates the source project silently."</p>
                    <ButtonGroup class="button-row">
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || busy.get() on:click=move |_| submit(
                            client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryScan
                        )>"Scan for recovery"</Button>
                        <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || fake_paths().is_none() || busy.get() on:click=move |_| {
                            if let Some(paths) = fake_paths() {
                                submit(client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryOpen { project_path: paths.project });
                            }
                        }>"Open recovered copy"</Button>
                        <Button variant=ButtonVariant::Destructive attr:r#type="button" attr:disabled=move || fake_paths().is_none() || busy.get() on:click=move |_| {
                            if let Some(paths) = fake_paths() {
                                submit(client, snapshot, status, error, busy, WindowRole::Recovery, IpcCommand::RecoveryDiscard { project_path: paths.project });
                            }
                        }>"Discard recovery copy"</Button>
                    </ButtonGroup>
                </Card>

                <Card attr:id="editor" attr:aria-labelledby="editor-heading">
                    <div class="section-heading">
                        <div>
                            <p class="eyebrow">"Revision fenced"</p>
                            <h2 id="editor-heading">"Editor and timeline"</h2>
                        </div>
                    </div>
                    <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || fake_paths().is_none() || busy.get() on:click=move |_| {
                        if let Some(paths) = fake_paths() {
                            submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::EditorOpen { project_path: paths.project });
                        }
                    }>"Open sample project"</Button>
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
                    </FieldGroup>
                    <ButtonGroup class="button-row">
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
                                        DesktopAdapterKind::NativeMacOs => state
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
                                            if let (Some(paths), EditorState::Ready { revision, .. }) = (fake_paths(), state.editor) {
                                                submit(client, snapshot, status, error, busy, WindowRole::Editor, IpcCommand::ExportStart {
                                                    project_revision: revision,
                                                    output_path: paths.export,
                                                    profile: ExportProfile::DistributionMp4,
                                                });
                                            }
                                        }
                                        DesktopAdapterKind::NativeMacOs => {
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
                                }>{move || if is_native() { "Export editable WebM" } else { "Start export" }}</Button>
                                <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || !snapshot.get().is_some_and(|state| matches!(state.export, ExportState::Running { .. })) || busy.get() on:click=move |_| {
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
                    <Show when=move || is_native()>
                        <div class="privacy-note" aria-labelledby="native-audio-heading">
                            <h3 id="native-audio-heading">"Native macOS system audio"</h3>
                            <p id="native-audio-help">
                                "System audio is optional and uses macOS Screen & System Audio Recording access. Frame excludes its own process audio. Microphone and camera remain off."
                            </p>
                            <Button variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:aria-describedby="native-audio-help"
                                attr:aria-pressed=move || snapshot
                                    .get()
                                    .is_some_and(|state| state.settings.system_audio_enabled)
                                attr:disabled=move || !can_configure_native_audio()
                                on:click=move |_| {
                                    if let Some(state) = snapshot.get_untracked() {
                                        submit(
                                            client,
                                            snapshot,
                                            status,
                                            error,
                                            busy,
                                            WindowRole::Settings,
                                            IpcCommand::SettingsApply {
                                                expected_revision: state.settings.revision,
                                                mode: state.settings.mode,
                                                frame_rate: state.settings.frame_rate,
                                                microphone_enabled: false,
                                                system_audio_enabled: !state.settings.system_audio_enabled,
                                                camera_enabled: false,
                                                reduced_motion: state.settings.reduced_motion,
                                            },
                                        );
                                    }
                                }
                            >{move || if snapshot.get().is_some_and(|state| state.settings.system_audio_enabled) {
                                "Include system audio: on"
                            } else {
                                "Include system audio: off"
                            }}</Button>
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
                            <p>"Legacy settings and projects are inspected read-only. The previous signed desktop remains selectable until parity gate 29 is approved."</p>
                        </Alert>
                    </aside>
                    <div class="split-grid">
                        <Card attr:aria-labelledby="lifecycle-heading">
                            <h3 id="lifecycle-heading">"Hotkeys, tray, and overlay"</h3>
                            <p>{move || snapshot.get().map_or("Lifecycle unavailable.", |state| {
                                if state.lifecycle.hotkeys_registered { "Global hotkeys registered by backend." } else { "Global hotkeys are not registered." }
                            })}</p>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || busy.get() on:click=move |_| submit(
                                client, snapshot, status, error, busy, WindowRole::Main,
                                IpcCommand::Lifecycle { action: LifecycleAction::RegisterHotkeys }
                            )>"Register fake hotkeys"</Button>
                        </Card>
                        <Card attr:aria-labelledby="update-heading">
                            <h3 id="update-heading">"Updates"</h3>
                            <p>{move || match snapshot.get().map(|state| state.update) {
                                Some(UpdateState::Current { .. }) => "Frame is current.",
                                Some(UpdateState::Available { .. }) => "An update is available.",
                                Some(UpdateState::ReadyToRelaunch { .. }) => "Update installed; relaunch is ready.",
                                None => "Update status unavailable.",
                            }}</p>
                            <Button variant=ButtonVariant::Outline attr:r#type="button" attr:disabled=move || !is_fake() || snapshot.get().is_none() || busy.get() on:click=move |_| {
                                if let Some(state) = snapshot.get_untracked() {
                                    let (action, expected_revision) = match state.update {
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
