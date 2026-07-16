#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::sync::Mutex;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use frame_desktop_core::{
    DesktopAdapterKind, DesktopBootstrap, DesktopDispatch, DesktopRoots, DesktopRuntime,
    PublicErrorCode, ShellCapabilities,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::{Emitter, Manager};

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct NativeDesktopState(Mutex<DesktopRuntime>);

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy, serde::Serialize)]
struct DesktopBoundaryError {
    code: PublicErrorCode,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main_window(label: &str) -> Result<(), &'static str> {
    if label == "main" {
        Ok(())
    } else {
        Err("window_not_authorized")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
fn bootstrap_main(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<ShellCapabilities, &'static str> {
    main_window(window.label())?;
    let capabilities = shell_capabilities();
    if std::env::var("FRAME_DESKTOP_SMOKE").as_deref() == Ok("1") {
        use std::io::Write;

        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "FRAME_DESKTOP_SMOKE_V1 protocol={} backend_truth={}",
            capabilities.protocol_version, capabilities.backend_truth
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
    main_window(window.label()).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Forbidden,
    })?;
    state
        .0
        .lock()
        .map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })
        .map(|runtime| runtime.bootstrap())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
fn dispatch_main(
    request_json: String,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<DesktopDispatch, DesktopBoundaryError> {
    main_window(window.label()).map_err(|_| DesktopBoundaryError {
        code: PublicErrorCode::Forbidden,
    })?;
    let dispatch = state
        .0
        .lock()
        .map_err(|_| DesktopBoundaryError {
            code: PublicErrorCode::Internal,
        })?
        .dispatch_json(&request_json)
        .map_err(|error| DesktopBoundaryError {
            code: error.public_code(),
        })?;
    for event in &dispatch.events {
        app.emit("frame-desktop://event-v1", event)
            .map_err(|_| DesktopBoundaryError {
                code: PublicErrorCode::Internal,
            })?;
    }
    Ok(dispatch)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn shell_capabilities() -> ShellCapabilities {
    ShellCapabilities::current()
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
    } else {
        DesktopAdapterKind::Unavailable
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let exports = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| data.join("exports"));
            let roots = DesktopRoots::new(
                data.join("projects").to_string_lossy(),
                data.join("media").to_string_lossy(),
                exports.to_string_lossy(),
            );
            let runtime = DesktopRuntime::new(configured_adapter(), roots, &session_nonce())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            app.manage(NativeDesktopState(Mutex::new(runtime)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap_main,
            bootstrap_desktop,
            dispatch_main
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
    fn commands_are_restricted_to_the_main_window() {
        assert_eq!(main_window("main"), Ok(()));
        assert_eq!(
            main_window("recorder-attacker"),
            Err("window_not_authorized")
        );
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
                "allow-dispatch-main"
            ])
        );
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
    }

    #[test]
    fn release_adapter_selection_is_fail_closed() {
        if !cfg!(debug_assertions) {
            assert_eq!(configured_adapter(), DesktopAdapterKind::Unavailable);
        }
    }

    #[test]
    fn shell_never_claims_an_unselected_capture_adapter() {
        let capabilities = shell_capabilities();
        assert_eq!(capabilities.protocol_version, 1);
        assert!(capabilities.is_current_backend_truth());
        assert_eq!(
            capabilities.recorder_adapter,
            frame_desktop_core::RecorderAdapterState::NotSelected
        );
        assert_eq!(
            capabilities.editor_adapter,
            frame_desktop_core::EditorAdapterState::RevisionFencedCore
        );
    }
}
