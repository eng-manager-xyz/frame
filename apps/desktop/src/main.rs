#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(all(target_os = "macos", feature = "macos-native"))]
use frame_desktop_core::MacOsNativeDesktopBackend;
#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
use frame_desktop_core::NativeDesktopBackend;
#[cfg(all(target_os = "windows", feature = "windows-native"))]
use frame_desktop_core::WindowsNativeDesktopBackend;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use frame_desktop_core::{
    DesktopAdapterKind, DesktopBootstrap, DesktopDispatch, DesktopRoots, DesktopRuntime,
    DesktopShellCommand, DesktopShellFailure, DesktopShellOutcome, DesktopShellStart,
    InstantFinalizeCommandV1, InstantFinalizeService, InstantFinalizeServiceError,
    InstantFinalizeUiUpdate, LifecycleAction, LifecycleSnapshot, PublicErrorCode,
    ShellCapabilities, UpdateAction, decode_request,
};
#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
use frame_legacy_import::{LegacyImportError, LegacyProjectMigrationService};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::{Emitter, Manager};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri_plugin_updater::{Update, UpdaterExt};

#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAX_INSTANT_FINALIZE_COMMAND_BYTES: usize = 512;
#[cfg(any(target_os = "macos", target_os = "windows"))]
const UPDATE_ENDPOINT: &str = "https://frame.engmanager.xyz/api/v1/desktop/updates/stable/{{target}}/{{arch}}/{{current_version}}?bundle={{bundle_type}}";
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PREVIOUS_UPDATE_ENDPOINT: &str = "https://frame.engmanager.xyz/api/v1/desktop/updates/previous/{{target}}/{{arch}}/{{current_version}}?bundle={{bundle_type}}";
#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAIN_WINDOW_LABEL: &str = "main";
#[cfg(any(target_os = "macos", target_os = "windows"))]
const OVERLAY_WINDOW_LABEL: &str = "overlay";
#[cfg(any(target_os = "macos", target_os = "windows"))]
const TARGET_PICKER_WINDOW_LABEL: &str = "target-picker";

#[cfg(all(target_os = "macos", feature = "macos-native"))]
mod hardware_driver;

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct NativeDesktopState {
    runtime: Mutex<DesktopRuntime>,
    #[cfg(all(target_os = "macos", feature = "macos-native"))]
    native_backend: Option<Mutex<MacOsNativeDesktopBackend>>,
    #[cfg(all(target_os = "windows", feature = "windows-native"))]
    native_backend: Option<Mutex<WindowsNativeDesktopBackend>>,
    #[cfg(any(
        all(target_os = "macos", feature = "macos-native"),
        all(target_os = "windows", feature = "windows-native")
    ))]
    legacy_migration: Option<Mutex<LegacyProjectMigrationService>>,
    instant_finalize: InstantFinalizeService,
    pending_update: Mutex<Option<Update>>,
    shell_busy: AtomicBool,
    frame_windows_excluded: bool,
    quitting: AtomicBool,
    tray: Mutex<Option<tauri::tray::TrayIcon>>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct ShellBusyGuard<'a>(&'a AtomicBool);

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Drop for ShellBusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn acquire_shell(state: &NativeDesktopState) -> Result<ShellBusyGuard<'_>, DesktopShellFailure> {
    state
        .shell_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ShellBusyGuard(&state.shell_busy))
        .map_err(|_| DesktopShellFailure::busy())
}

#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
fn map_legacy_import_error(error: LegacyImportError) -> DesktopShellFailure {
    match error {
        LegacyImportError::StaleCatalog
        | LegacyImportError::NeedsReview
        | LegacyImportError::Unsupported
        | LegacyImportError::SourceChanged => DesktopShellFailure::conflict(),
        LegacyImportError::Unavailable | LegacyImportError::InvalidProject => {
            DesktopShellFailure::unavailable()
        }
        LegacyImportError::Bound
        | LegacyImportError::InvalidCatalog
        | LegacyImportError::Storage
        | LegacyImportError::RandomUnavailable => DesktopShellFailure::internal(),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct DesktopBoundaryError {
    code: PublicErrorCode,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main_window(label: &str) -> Result<(), &'static str> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("window_not_authorized")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn frame_window(label: &str) -> Result<(), &'static str> {
    if known_frame_window(label) {
        Ok(())
    } else {
        Err("window_not_authorized")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn physical_window_allows(label: &str, role: frame_desktop_core::WindowRole) -> bool {
    use frame_desktop_core::WindowRole;

    match label {
        MAIN_WINDOW_LABEL => !matches!(role, WindowRole::Overlay | WindowRole::TargetPicker),
        OVERLAY_WINDOW_LABEL => role == WindowRole::Overlay,
        TARGET_PICKER_WINDOW_LABEL => role == WindowRole::TargetPicker,
        _ => false,
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn known_frame_window(label: &str) -> bool {
    matches!(
        label,
        MAIN_WINDOW_LABEL | OVERLAY_WINDOW_LABEL | TARGET_PICKER_WINDOW_LABEL
    )
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn updater_public_key() -> Option<&'static str> {
    option_env!("FRAME_TAURI_UPDATER_PUBLIC_KEY").filter(|value| !value.trim().is_empty())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn shell_shortcuts() -> [Shortcut; 3] {
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(target_os = "windows")]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;
    [
        Shortcut::new(Some(modifiers), Code::Digit1),
        Shortcut::new(Some(modifiers), Code::Digit2),
        Shortcut::new(Some(modifiers), Code::Digit3),
    ]
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn all_shortcuts_registered(app: &tauri::AppHandle) -> bool {
    shell_shortcuts()
        .into_iter()
        .all(|shortcut| app.global_shortcut().is_registered(shortcut))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn window_is_visible(app: &tauri::AppHandle, label: &str) -> bool {
    app.get_webview_window(label)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn lifecycle_snapshot(app: &tauri::AppHandle, state: &NativeDesktopState) -> LifecycleSnapshot {
    LifecycleSnapshot {
        main_visible: window_is_visible(app, MAIN_WINDOW_LABEL),
        overlay_visible: window_is_visible(app, OVERLAY_WINDOW_LABEL),
        target_picker_visible: window_is_visible(app, TARGET_PICKER_WINDOW_LABEL),
        hotkeys_registered: all_shortcuts_registered(app),
        frame_windows_excluded: state.frame_windows_excluded,
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn require_window(
    app: &tauri::AppHandle,
    label: &str,
) -> Result<tauri::WebviewWindow, DesktopShellFailure> {
    app.get_webview_window(label)
        .ok_or_else(DesktopShellFailure::unavailable)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn show_window(
    app: &tauri::AppHandle,
    label: &str,
    focus: bool,
) -> Result<(), DesktopShellFailure> {
    let window = require_window(app, label)?;
    position_auxiliary_window(app, &window);
    window.show().map_err(|_| DesktopShellFailure::internal())?;
    if focus {
        window
            .set_focus()
            .map_err(|_| DesktopShellFailure::internal())?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn position_auxiliary_window(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    if window.label() == MAIN_WINDOW_LABEL {
        return;
    }
    let monitor = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .and_then(|main| main.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let Ok(window_size) = window.outer_size() else {
        return;
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let x = monitor_position.x
        + i32::try_from(monitor_size.width.saturating_sub(window_size.width) / 2).unwrap_or(0);
    let vertical_offset = if window.label() == OVERLAY_WINDOW_LABEL {
        48
    } else {
        i32::try_from(monitor_size.height.saturating_sub(window_size.height) / 2).unwrap_or(0)
    };
    let y = monitor_position.y + vertical_offset;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn hide_window(app: &tauri::AppHandle, label: &str) -> Result<(), DesktopShellFailure> {
    require_window(app, label)?
        .hide()
        .map_err(|_| DesktopShellFailure::internal())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn apply_lifecycle(
    app: &tauri::AppHandle,
    state: &NativeDesktopState,
    action: LifecycleAction,
    invoking_window: &str,
) -> Result<LifecycleSnapshot, DesktopShellFailure> {
    match action {
        LifecycleAction::RegisterHotkeys => {
            let missing = shell_shortcuts()
                .into_iter()
                .filter(|shortcut| !app.global_shortcut().is_registered(*shortcut))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                app.global_shortcut()
                    .register_multiple(missing)
                    .map_err(|_| DesktopShellFailure::unavailable())?;
            }
        }
        LifecycleAction::ShowMainWindow | LifecycleAction::ReopenWindow => {
            show_window(app, MAIN_WINDOW_LABEL, true)?;
        }
        LifecycleAction::HideMainWindow => hide_window(app, MAIN_WINDOW_LABEL)?,
        LifecycleAction::ShowOverlay => show_window(app, OVERLAY_WINDOW_LABEL, true)?,
        LifecycleAction::HideOverlay => hide_window(app, OVERLAY_WINDOW_LABEL)?,
        LifecycleAction::ShowTargetPicker => {
            show_window(app, TARGET_PICKER_WINDOW_LABEL, true)?;
        }
        LifecycleAction::HideTargetPicker => hide_window(app, TARGET_PICKER_WINDOW_LABEL)?,
        LifecycleAction::CloseWindow => {
            if !known_frame_window(invoking_window) {
                return Err(DesktopShellFailure::unavailable());
            }
            hide_window(app, invoking_window)?;
        }
    }
    Ok(lifecycle_snapshot(app, state))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn emit_desktop_events(app: &tauri::AppHandle, dispatch: &DesktopDispatch) {
    for event in &dispatch.events {
        let _ = app.emit("frame-desktop://event-v1", event);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn observe_lifecycle(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<NativeDesktopState>() else {
        return;
    };
    let snapshot = lifecycle_snapshot(app, &state);
    let Ok(mut runtime) = state.runtime.lock() else {
        return;
    };
    let Ok(events) = runtime.observe_shell_lifecycle(snapshot) else {
        return;
    };
    drop(runtime);
    for event in events {
        let _ = app.emit("frame-desktop://event-v1", event);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn apply_os_lifecycle(app: &tauri::AppHandle, action: LifecycleAction, invoking_window: &str) {
    let Some(state) = app.try_state::<NativeDesktopState>() else {
        return;
    };
    if apply_lifecycle(app, &state, action, invoking_window).is_ok() {
        observe_lifecycle(app);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn handle_shortcut(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let shortcuts = shell_shortcuts();
    let action = if shortcut.id() == shortcuts[0].id() {
        if window_is_visible(app, MAIN_WINDOW_LABEL) {
            LifecycleAction::HideMainWindow
        } else {
            LifecycleAction::ShowMainWindow
        }
    } else if shortcut.id() == shortcuts[1].id() {
        if window_is_visible(app, TARGET_PICKER_WINDOW_LABEL) {
            LifecycleAction::HideTargetPicker
        } else {
            LifecycleAction::ShowTargetPicker
        }
    } else if shortcut.id() == shortcuts[2].id() {
        if window_is_visible(app, OVERLAY_WINDOW_LABEL) {
            LifecycleAction::HideOverlay
        } else {
            LifecycleAction::ShowOverlay
        }
    } else {
        return;
    };
    apply_os_lifecycle(app, action, MAIN_WINDOW_LABEL);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn install_tray(app: &tauri::AppHandle) -> Result<tauri::tray::TrayIcon, tauri::Error> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };

    let show = MenuItem::with_id(app, "frame-show-main", "Show Frame", true, None::<&str>)?;
    let target = MenuItem::with_id(
        app,
        "frame-show-target-picker",
        "Choose capture target",
        true,
        None::<&str>,
    )?;
    let overlay = MenuItem::with_id(
        app,
        "frame-show-overlay",
        "Show recording controls",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "frame-quit", "Quit Frame", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &target, &overlay, &quit])?;
    let mut builder = TrayIconBuilder::with_id("frame")
        .menu(&menu)
        .tooltip("Frame screen recorder")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "frame-show-main" => {
                apply_os_lifecycle(app, LifecycleAction::ShowMainWindow, MAIN_WINDOW_LABEL);
            }
            "frame-show-target-picker" => {
                apply_os_lifecycle(app, LifecycleAction::ShowTargetPicker, MAIN_WINDOW_LABEL);
            }
            "frame-show-overlay" => {
                apply_os_lifecycle(app, LifecycleAction::ShowOverlay, MAIN_WINDOW_LABEL);
            }
            "frame-quit" => {
                if let Some(state) = app.try_state::<NativeDesktopState>() {
                    state.quitting.store(true, Ordering::Release);
                }
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn execute_shell(
    app: &tauri::AppHandle,
    state: &NativeDesktopState,
    command: DesktopShellCommand,
    invoking_window: &str,
) -> Result<DesktopShellOutcome, DesktopShellFailure> {
    match command {
        DesktopShellCommand::Lifecycle { action } => {
            let snapshot = apply_lifecycle(app, state, action, invoking_window)?;
            Ok(DesktopShellOutcome::LifecycleApplied { snapshot })
        }
        DesktopShellCommand::Update {
            action: UpdateAction::Check | UpdateAction::CheckPrevious,
            ..
        } => {
            let public_key = updater_public_key().ok_or_else(DesktopShellFailure::unavailable)?;
            let previous = matches!(
                command,
                DesktopShellCommand::Update {
                    action: UpdateAction::CheckPrevious,
                    ..
                }
            );
            let endpoint = if previous {
                PREVIOUS_UPDATE_ENDPOINT
            } else {
                UPDATE_ENDPOINT
            }
            .parse()
            .map_err(|_| DesktopShellFailure::internal())?;
            let mut builder = app
                .updater_builder()
                .pubkey(public_key)
                .endpoints(vec![endpoint])
                .map_err(|_| DesktopShellFailure::internal())?
                .timeout(Duration::from_secs(30));
            if previous {
                builder = builder.version_comparator(|current, release| release.version < current);
            }
            let updater = builder
                .build()
                .map_err(|_| DesktopShellFailure::internal())?;
            let update = updater
                .check()
                .await
                .map_err(|_| DesktopShellFailure::unavailable())?;
            let available = update.is_some();
            *state
                .pending_update
                .lock()
                .map_err(|_| DesktopShellFailure::internal())? = update;
            Ok(DesktopShellOutcome::UpdateChecked { available })
        }
        DesktopShellCommand::Update {
            action: UpdateAction::Install | UpdateAction::InstallPrevious,
            ..
        } => {
            let update = state
                .pending_update
                .lock()
                .map_err(|_| DesktopShellFailure::internal())?
                .clone()
                .ok_or_else(DesktopShellFailure::conflict)?;
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|_| DesktopShellFailure::unavailable())?;
            Ok(DesktopShellOutcome::UpdateInstalled)
        }
        DesktopShellCommand::Update {
            action: UpdateAction::Relaunch,
            ..
        } => Ok(DesktopShellOutcome::RelaunchRequested),
        DesktopShellCommand::LegacyProjectScan => {
            #[cfg(any(
                all(target_os = "macos", feature = "macos-native"),
                all(target_os = "windows", feature = "windows-native")
            ))]
            {
                let catalog = state
                    .legacy_migration
                    .as_ref()
                    .ok_or_else(DesktopShellFailure::unavailable)?
                    .lock()
                    .map_err(|_| DesktopShellFailure::internal())?
                    .scan()
                    .map_err(map_legacy_import_error)?;
                Ok(DesktopShellOutcome::LegacyProjectsScanned { catalog })
            }
            #[cfg(not(any(
                all(target_os = "macos", feature = "macos-native"),
                all(target_os = "windows", feature = "windows-native")
            )))]
            {
                Err(DesktopShellFailure::unavailable())
            }
        }
        DesktopShellCommand::LegacyProjectImport {
            catalog_generation,
            project_token,
        } => {
            #[cfg(any(
                all(target_os = "macos", feature = "macos-native"),
                all(target_os = "windows", feature = "windows-native")
            ))]
            {
                let (receipt, catalog) = {
                    let mut migration = state
                        .legacy_migration
                        .as_ref()
                        .ok_or_else(DesktopShellFailure::unavailable)?
                        .lock()
                        .map_err(|_| DesktopShellFailure::internal())?;
                    let receipt = migration
                        .import(catalog_generation, &project_token)
                        .map_err(map_legacy_import_error)?;
                    let catalog = migration.scan().map_err(map_legacy_import_error)?;
                    (receipt, catalog)
                };
                let studio_projects = state
                    .native_backend
                    .as_ref()
                    .ok_or_else(DesktopShellFailure::unavailable)?
                    .lock()
                    .map_err(|_| DesktopShellFailure::internal())?
                    .scan_studio_projects()
                    .map_err(|_| DesktopShellFailure::internal())?;
                Ok(DesktopShellOutcome::LegacyProjectImported {
                    catalog,
                    receipt,
                    studio_projects,
                })
            }
            #[cfg(not(any(
                all(target_os = "macos", feature = "macos-native"),
                all(target_os = "windows", feature = "windows-native")
            )))]
            {
                let _ = (catalog_generation, project_token);
                Err(DesktopShellFailure::unavailable())
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
fn bootstrap_main(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<ShellCapabilities, &'static str> {
    frame_window(window.label())?;
    let adapter = state
        .runtime
        .lock()
        .map_err(|_| "desktop_runtime_unavailable")?
        .snapshot()
        .adapter;
    let capabilities = shell_capabilities(adapter, state.instant_finalize.capability());
    if std::env::var("FRAME_DESKTOP_SMOKE").as_deref() == Ok("1") {
        use std::io::Write;

        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "FRAME_DESKTOP_SMOKE_V1 protocol={} backend_truth={} recorder_adapter={}",
            capabilities.protocol_version,
            capabilities.backend_truth,
            match capabilities.recorder_adapter {
                frame_desktop_core::RecorderAdapterState::Unavailable => "unavailable",
                frame_desktop_core::RecorderAdapterState::DeterministicFake => {
                    "deterministic_fake"
                }
                frame_desktop_core::RecorderAdapterState::NativeMacOsDisplay => {
                    "native_macos_display"
                }
                frame_desktop_core::RecorderAdapterState::NativeWindowsDisplayWindowRegion => {
                    "native_windows_display_window_region"
                }
            }
        )
        .expect("desktop smoke marker write failed");
        stdout.flush().expect("desktop smoke marker flush failed");
        drop(stdout);
        app.exit(0);
    }
    Ok(capabilities)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
fn bootstrap_desktop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<DesktopBootstrap, DesktopBoundaryError> {
    frame_window(window.label()).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Forbidden,
    })?;
    let mut bootstrap = state
        .runtime
        .lock()
        .map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })
        .map(|runtime| runtime.bootstrap())?;
    bootstrap
        .contexts
        .retain(|context| physical_window_allows(window.label(), context.role));
    Ok(bootstrap)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
async fn dispatch_main(
    request_json: String,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<DesktopDispatch, DesktopBoundaryError> {
    frame_window(window.label()).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Forbidden,
    })?;
    let request = decode_request(&request_json).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::InvalidRequest,
    })?;
    let adapter = {
        let runtime = state.runtime.lock().map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })?;
        let bootstrap = runtime.bootstrap();
        let request_role = bootstrap
            .contexts
            .iter()
            .find(|context| context.window_id == request.window_id)
            .map(|context| context.role)
            .ok_or(DesktopBoundaryError {
                code: PublicErrorCode::Forbidden,
            })?;
        if !physical_window_allows(window.label(), request_role) {
            return Err(DesktopBoundaryError {
                code: PublicErrorCode::Forbidden,
            });
        }
        runtime.snapshot().adapter
    };

    if adapter != DesktopAdapterKind::DeterministicFake
        && matches!(
            &request.command,
            frame_desktop_core::IpcCommand::Lifecycle { .. }
                | frame_desktop_core::IpcCommand::Update { .. }
                | frame_desktop_core::IpcCommand::LegacyProjectScan
                | frame_desktop_core::IpcCommand::LegacyProjectImport { .. }
        )
    {
        let start = state
            .runtime
            .lock()
            .map_err(|_| DesktopBoundaryError {
                code: PublicErrorCode::Internal,
            })?
            .begin_shell(request)
            .map_err(|error| DesktopBoundaryError {
                code: error.public_code(),
            })?;
        let pending = match start {
            DesktopShellStart::Complete(dispatch) => {
                emit_desktop_events(&app, &dispatch);
                return Ok(*dispatch);
            }
            DesktopShellStart::Pending(pending) => pending,
        };
        let outcome = match acquire_shell(&state) {
            Ok(_guard) => execute_shell(&app, &state, pending.command(), window.label()).await,
            Err(error) => Err(error),
        };
        let completion = state
            .runtime
            .lock()
            .map_err(|_| DesktopBoundaryError {
                code: PublicErrorCode::Internal,
            })?
            .finish_shell(pending, outcome)
            .map_err(|error| DesktopBoundaryError {
                code: error.public_code(),
            })?;
        emit_desktop_events(&app, &completion.dispatch);
        if completion.restart_requested {
            app.request_restart();
        }
        return Ok(completion.dispatch);
    }

    let mut runtime = state.runtime.lock().map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Internal,
    })?;
    #[cfg(any(
        all(target_os = "macos", feature = "macos-native"),
        all(target_os = "windows", feature = "windows-native")
    ))]
    let dispatch = if matches!(
        runtime.snapshot().adapter,
        DesktopAdapterKind::NativeMacOs | DesktopAdapterKind::NativeWindows
    ) {
        let backend = state.native_backend.as_ref().ok_or(DesktopBoundaryError {
            code: PublicErrorCode::Unavailable,
        })?;
        runtime.dispatch_native_json(
            &request_json,
            &mut *backend.lock().map_err(|_| DesktopBoundaryError {
                code: PublicErrorCode::Internal,
            })?,
        )
    } else {
        runtime.dispatch_json(&request_json)
    }
    .map_err(|error| DesktopBoundaryError {
        code: error.public_code(),
    })?;
    #[cfg(any(
        all(target_os = "windows", not(feature = "windows-native")),
        all(target_os = "macos", not(feature = "macos-native"))
    ))]
    let dispatch = runtime
        .dispatch_json(&request_json)
        .map_err(|error| DesktopBoundaryError {
            code: error.public_code(),
        })?;
    emit_desktop_events(&app, &dispatch);
    Ok(dispatch)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn decode_instant_finalize_command(
    command_json: &str,
) -> Result<InstantFinalizeCommandV1, DesktopBoundaryError> {
    if command_json.is_empty() || command_json.len() > MAX_INSTANT_FINALIZE_COMMAND_BYTES {
        return Err(DesktopBoundaryError {
            code: PublicErrorCode::InvalidRequest,
        });
    }
    let command = serde_json::from_str::<InstantFinalizeCommandV1>(command_json).map_err(|_| {
        DesktopBoundaryError {
            code: PublicErrorCode::InvalidRequest,
        }
    })?;
    command.validate().map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::InvalidRequest,
    })?;
    Ok(command)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn instant_finalize_error(error: InstantFinalizeServiceError) -> DesktopBoundaryError {
    let code = match error {
        InstantFinalizeServiceError::InvalidEnvelope => PublicErrorCode::InvalidRequest,
        InstantFinalizeServiceError::Unavailable => PublicErrorCode::Unavailable,
        InstantFinalizeServiceError::UnknownHandle => PublicErrorCode::Forbidden,
        InstantFinalizeServiceError::Busy => PublicErrorCode::Busy,
        InstantFinalizeServiceError::SequenceReplay
        | InstantFinalizeServiceError::SequenceGap
        | InstantFinalizeServiceError::AuthorityChanged
        | InstantFinalizeServiceError::Terminal => PublicErrorCode::Conflict,
        InstantFinalizeServiceError::ProviderRejected
        | InstantFinalizeServiceError::RandomUnavailable
        | InstantFinalizeServiceError::RegistryUnavailable => PublicErrorCode::Internal,
    };
    DesktopBoundaryError { code }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn require_instant_finalize_available(
    service: &InstantFinalizeService,
) -> Result<(), DesktopBoundaryError> {
    if service.capability() == frame_desktop_core::InstantFinalizeCapabilityState::Available {
        Ok(())
    } else {
        Err(DesktopBoundaryError {
            code: PublicErrorCode::Unavailable,
        })
    }
}

/// The authorization check intentionally precedes JSON parsing. A non-main
/// WebView cannot use deserialization behavior as a command oracle.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
async fn finalize_instant(
    command_json: String,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<InstantFinalizeUiUpdate, DesktopBoundaryError> {
    main_window(window.label()).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Forbidden,
    })?;
    require_instant_finalize_available(&state.instant_finalize)?;
    let command = decode_instant_finalize_command(&command_json)?;
    let handle = command.handle.clone();
    let command_sequence = command.sequence;
    state
        .runtime
        .lock()
        .map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })?
        .preflight_instant_finalize(&handle, command_sequence)
        .map_err(|error| DesktopBoundaryError {
            code: error.public_code(),
        })?;

    // Reconcile a result committed by the service if an earlier Tauri future
    // was cancelled after network completion but before the runtime update.
    // Otherwise dispatch without holding the runtime registry lock.
    let result = match state.instant_finalize.reconciled_result(&command) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => state.instant_finalize.dispatch(command).await,
        Err(error) => Err(error),
    };
    let result = match result {
        Ok(result) => result,
        Err(
            error @ (InstantFinalizeServiceError::ProviderRejected
            | InstantFinalizeServiceError::Terminal),
        ) => {
            let update = state
                .runtime
                .lock()
                .map_err(|_| DesktopBoundaryError {
                    code: PublicErrorCode::Internal,
                })?
                .disable_native_instant_finalize(&handle, command_sequence)
                .map_err(|runtime_error| DesktopBoundaryError {
                    code: runtime_error.public_code(),
                })?;
            let _ = state.instant_finalize.forget_terminal_context(&handle);
            emit_instant_update(&app, &update);
            debug_assert!(matches!(
                error,
                InstantFinalizeServiceError::ProviderRejected
                    | InstantFinalizeServiceError::Terminal
            ));
            return Ok(update);
        }
        Err(error) => return Err(instant_finalize_error(error)),
    };
    let update = state
        .runtime
        .lock()
        .map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })?
        .apply_instant_finalize_progress(&handle, result.sequence, result.progress)
        .map_err(|error| DesktopBoundaryError {
            code: error.public_code(),
        })?;
    if matches!(
        result.progress.phase,
        frame_client::InstantUiPhaseV1::ShareReady
            | frame_client::InstantUiPhaseV1::Cancelled
            | frame_client::InstantUiPhaseV1::RecoveryRequired
    ) {
        let _ = state.instant_finalize.forget_terminal_context(&handle);
    }
    emit_instant_update(&app, &update);
    Ok(update)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn emit_instant_update(app: &tauri::AppHandle, update: &InstantFinalizeUiUpdate) {
    for event in &update.events {
        let _ = app.emit("frame-desktop://event-v1", event);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn shell_capabilities(
    adapter: DesktopAdapterKind,
    instant_finalize: frame_desktop_core::InstantFinalizeCapabilityState,
) -> ShellCapabilities {
    let recorder_adapter = match adapter {
        DesktopAdapterKind::Unavailable => frame_desktop_core::RecorderAdapterState::Unavailable,
        DesktopAdapterKind::DeterministicFake => {
            frame_desktop_core::RecorderAdapterState::DeterministicFake
        }
        DesktopAdapterKind::NativeMacOs => {
            frame_desktop_core::RecorderAdapterState::NativeMacOsDisplay
        }
        DesktopAdapterKind::NativeWindows => {
            frame_desktop_core::RecorderAdapterState::NativeWindowsDisplayWindowRegion
        }
    };
    ShellCapabilities {
        recorder_adapter,
        instant_finalize,
        ..ShellCapabilities::current()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn session_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{elapsed}", std::process::id())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn configured_adapter() -> DesktopAdapterKind {
    if cfg!(debug_assertions) && std::env::var("FRAME_DESKTOP_FAKE_PIPELINE").as_deref() == Ok("1")
    {
        DesktopAdapterKind::DeterministicFake
    } else if cfg!(all(target_os = "macos", feature = "macos-native")) {
        DesktopAdapterKind::NativeMacOs
    } else if cfg!(all(target_os = "windows", feature = "windows-native")) {
        DesktopAdapterKind::NativeWindows
    } else {
        DesktopAdapterKind::Unavailable
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    #[cfg(all(target_os = "macos", feature = "macos-native"))]
    if let Some(result) = hardware_driver::run_if_requested() {
        if let Err(error) = result {
            eprintln!("Frame protected hardware driver failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(any(
        all(target_os = "macos", feature = "macos-native"),
        all(target_os = "windows", feature = "windows-native")
    ))]
    if matches!(
        configured_adapter(),
        DesktopAdapterKind::NativeMacOs | DesktopAdapterKind::NativeWindows
    ) && let Err(error) = frame_desktop_core::bootstrap_desktop_gstreamer()
    {
        eprintln!("Frame desktop GStreamer bootstrap failed: {error}");
        std::process::exit(78);
    }

    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                handle_shortcut(app, shortcut);
            }
        })
        .build();
    let updater_plugin = updater_public_key().map_or_else(
        || tauri_plugin_updater::Builder::new().build(),
        |public_key| {
            tauri_plugin_updater::Builder::new()
                .pubkey(public_key)
                .build()
        },
    );

    tauri::Builder::default()
        .plugin(shortcut_plugin)
        .plugin(updater_plugin)
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            // Do not touch a TCC-protected user folder while Tauri is still
            // launching. The current automatic export is an app-owned
            // artifact; a future Save dialog can grant a user-selected path.
            let exports = data.join("exports");
            let roots = DesktopRoots::new(
                data.join("projects").to_string_lossy(),
                data.join("media").to_string_lossy(),
                exports.to_string_lossy(),
            );
            let requested_adapter = configured_adapter();
            let frame_windows_excluded = [
                MAIN_WINDOW_LABEL,
                OVERLAY_WINDOW_LABEL,
                TARGET_PICKER_WINDOW_LABEL,
            ]
            .into_iter()
            .all(|label| {
                app.get_webview_window(label)
                    .is_some_and(|window| window.set_content_protected(true).is_ok())
            });
            let hotkeys_registered = app
                .global_shortcut()
                .register_multiple(shell_shortcuts())
                .is_ok();
            #[cfg(all(target_os = "macos", feature = "macos-native"))]
            let (adapter, native_backend) = if requested_adapter == DesktopAdapterKind::NativeMacOs
            {
                match MacOsNativeDesktopBackend::new(
                    data.join("projects"),
                    data.join("media"),
                    exports,
                ) {
                    Ok(backend) => (DesktopAdapterKind::NativeMacOs, Some(Mutex::new(backend))),
                    Err(error) => {
                        eprintln!("Frame native capture adapter is unavailable: {error}");
                        (DesktopAdapterKind::Unavailable, None)
                    }
                }
            } else {
                (requested_adapter, None)
            };
            #[cfg(all(target_os = "windows", feature = "windows-native"))]
            let (adapter, native_backend) = if requested_adapter
                == DesktopAdapterKind::NativeWindows
            {
                match WindowsNativeDesktopBackend::new(
                    data.join("media"),
                    exports,
                    frame_windows_excluded,
                ) {
                    Ok(backend) => (DesktopAdapterKind::NativeWindows, Some(Mutex::new(backend))),
                    Err(error) => {
                        eprintln!("Frame native capture adapter is unavailable: {error}");
                        (DesktopAdapterKind::Unavailable, None)
                    }
                }
            } else {
                (requested_adapter, None)
            };
            #[cfg(any(
                all(target_os = "macos", feature = "macos-native"),
                all(target_os = "windows", feature = "windows-native")
            ))]
            let legacy_migration = data
                .parent()
                .map(|base| {
                    LegacyProjectMigrationService::new(
                        base.join("so.cap.desktop"),
                        data.join("projects"),
                        data.join("media").join("studio"),
                    )
                })
                .and_then(Result::ok)
                .map(Mutex::new);
            #[cfg(any(
                all(target_os = "windows", not(feature = "windows-native")),
                all(target_os = "macos", not(feature = "macos-native"))
            ))]
            let adapter = requested_adapter;
            let mut runtime = DesktopRuntime::new(adapter, roots, &session_nonce())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            runtime
                .initialize_shell_capabilities(
                    frame_windows_excluded,
                    updater_public_key().is_some(),
                )
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            if hotkeys_registered {
                let startup_lifecycle = LifecycleSnapshot {
                    main_visible: true,
                    overlay_visible: false,
                    target_picker_visible: false,
                    hotkeys_registered: true,
                    frame_windows_excluded,
                };
                runtime
                    .observe_shell_lifecycle(startup_lifecycle)
                    .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            }
            app.manage(NativeDesktopState {
                runtime: Mutex::new(runtime),
                #[cfg(all(target_os = "macos", feature = "macos-native"))]
                native_backend,
                #[cfg(all(target_os = "windows", feature = "windows-native"))]
                native_backend,
                #[cfg(any(
                    all(target_os = "macos", feature = "macos-native"),
                    all(target_os = "windows", feature = "windows-native")
                ))]
                legacy_migration,
                instant_finalize: InstantFinalizeService::not_configured(),
                pending_update: Mutex::new(None),
                shell_busy: AtomicBool::new(false),
                frame_windows_excluded,
                quitting: AtomicBool::new(false),
                tray: Mutex::new(None),
            });
            match install_tray(app.handle()) {
                Ok(tray) => {
                    if let Ok(mut slot) = app.state::<NativeDesktopState>().tray.lock() {
                        *slot = Some(tray);
                    }
                }
                Err(error) => eprintln!("Frame tray integration is unavailable: {error}"),
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let should_hide = app
                    .try_state::<NativeDesktopState>()
                    .is_none_or(|state| !state.quitting.load(Ordering::Acquire));
                if should_hide && known_frame_window(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                    observe_lifecycle(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap_main,
            bootstrap_desktop,
            dispatch_main,
            finalize_instant
        ])
        .run(tauri::generate_context!())
        .expect("Frame desktop shell failed");
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("Frame desktop is supported on macOS and Windows");
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn privileged_finalize_is_main_only_and_product_windows_are_explicit() {
        assert_eq!(main_window("main"), Ok(()));
        assert_eq!(main_window("overlay"), Err("window_not_authorized"));
        assert_eq!(
            main_window("recorder-attacker"),
            Err("window_not_authorized")
        );
        for label in ["main", "overlay", "target-picker"] {
            assert_eq!(frame_window(label), Ok(()));
        }
        assert_eq!(
            frame_window("recorder-attacker"),
            Err("window_not_authorized")
        );
    }

    #[test]
    fn physical_windows_cannot_borrow_another_surfaces_logical_authority() {
        use frame_desktop_core::WindowRole;

        assert!(physical_window_allows("main", WindowRole::Recorder));
        assert!(!physical_window_allows("main", WindowRole::Overlay));
        assert!(!physical_window_allows("main", WindowRole::TargetPicker));
        assert!(physical_window_allows("overlay", WindowRole::Overlay));
        assert!(!physical_window_allows("overlay", WindowRole::Recorder));
        assert!(physical_window_allows(
            "target-picker",
            WindowRole::TargetPicker
        ));
        assert!(!physical_window_allows("target-picker", WindowRole::Main));
        assert!(!physical_window_allows("unknown", WindowRole::Main));
    }

    #[test]
    fn capability_grants_only_versioned_bootstrap_and_dispatch() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json"))
                .expect("checked-in capability must be valid JSON");
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "allow-bootstrap-main",
                "allow-bootstrap-desktop",
                "allow-dispatch-main",
                "allow-finalize-instant"
            ])
        );
        assert_eq!(
            capability["windows"],
            serde_json::json!(["main", "overlay", "target-picker"])
        );
        let permissions = capability["permissions"]
            .as_array()
            .expect("permissions are an array");
        assert!(permissions.iter().all(|permission| {
            !permission.as_str().is_some_and(|value| {
                value.starts_with("updater:") || value.starts_with("global-shortcut:")
            })
        }));
    }

    #[test]
    fn updater_endpoint_is_same_origin_tls_and_has_no_embedded_authority() {
        assert!(UPDATE_ENDPOINT.starts_with("https://frame.engmanager.xyz/"));
        assert!(UPDATE_ENDPOINT.contains("{{target}}"));
        assert!(UPDATE_ENDPOINT.contains("{{arch}}"));
        assert!(UPDATE_ENDPOINT.contains("{{current_version}}"));
        assert!(!UPDATE_ENDPOINT.contains('@'));
        assert!(PREVIOUS_UPDATE_ENDPOINT.starts_with("https://frame.engmanager.xyz/"));
        assert!(PREVIOUS_UPDATE_ENDPOINT.contains("/updates/previous/"));
        assert!(PREVIOUS_UPDATE_ENDPOINT.contains("{{current_version}}"));
        assert!(!PREVIOUS_UPDATE_ENDPOINT.contains('@'));
    }

    #[test]
    fn release_adapter_selection_is_platform_truthful() {
        if !cfg!(debug_assertions) {
            #[cfg(all(target_os = "macos", feature = "macos-native"))]
            assert_eq!(configured_adapter(), DesktopAdapterKind::NativeMacOs);
            #[cfg(all(target_os = "windows", feature = "windows-native"))]
            assert_eq!(configured_adapter(), DesktopAdapterKind::NativeWindows);
            #[cfg(any(
                all(target_os = "windows", not(feature = "windows-native")),
                all(target_os = "macos", not(feature = "macos-native"))
            ))]
            assert_eq!(configured_adapter(), DesktopAdapterKind::Unavailable);
        }
    }

    #[test]
    fn shell_reports_the_runtime_capture_adapter() {
        let capabilities = shell_capabilities(
            DesktopAdapterKind::Unavailable,
            frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured,
        );
        assert_eq!(
            capabilities.protocol_version,
            frame_desktop_core::IPC_PROTOCOL_VERSION
        );
        assert!(capabilities.is_current_backend_truth());
        assert_eq!(
            capabilities.recorder_adapter,
            frame_desktop_core::RecorderAdapterState::Unavailable
        );
        assert_eq!(
            capabilities.editor_adapter,
            frame_desktop_core::EditorAdapterState::RevisionFencedCore
        );
        assert_eq!(
            capabilities.instant_finalize,
            frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured
        );

        assert_eq!(
            shell_capabilities(
                DesktopAdapterKind::DeterministicFake,
                frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured,
            )
            .recorder_adapter,
            frame_desktop_core::RecorderAdapterState::DeterministicFake
        );
        assert_eq!(
            shell_capabilities(
                DesktopAdapterKind::NativeMacOs,
                frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured,
            )
            .recorder_adapter,
            frame_desktop_core::RecorderAdapterState::NativeMacOsDisplay
        );
        assert_eq!(
            shell_capabilities(
                DesktopAdapterKind::NativeWindows,
                frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured,
            )
            .recorder_adapter,
            frame_desktop_core::RecorderAdapterState::NativeWindowsDisplayWindowRegion
        );
    }

    #[test]
    fn finalize_decoder_is_bounded_and_rejects_extra_authority_fields() {
        assert_eq!(
            decode_instant_finalize_command(&"x".repeat(MAX_INSTANT_FINALIZE_COMMAND_BYTES + 1)),
            Err(DesktopBoundaryError {
                code: PublicErrorCode::InvalidRequest
            })
        );
        let forbidden = format!(
            r#"{{"protocol_version":1,"action":"finalize","sequence":1,"handle":"{}","bearer":"forbidden"}}"#,
            "a".repeat(64)
        );
        assert_eq!(
            decode_instant_finalize_command(&forbidden),
            Err(DesktopBoundaryError {
                code: PublicErrorCode::InvalidRequest
            })
        );
    }

    #[test]
    fn release_finalize_provider_is_explicitly_unavailable() {
        let service = InstantFinalizeService::not_configured();
        assert_eq!(
            require_instant_finalize_available(&service),
            Err(DesktopBoundaryError {
                code: PublicErrorCode::Unavailable,
            })
        );
    }
}
