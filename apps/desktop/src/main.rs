#[cfg(any(target_os = "macos", target_os = "windows"))]
use frame_desktop_core::ShellCapabilities;

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
fn shell_capabilities() -> ShellCapabilities {
    ShellCapabilities::current()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![bootstrap_main])
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
    fn capability_grants_only_the_bootstrap_command() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json"))
                .expect("checked-in capability must be valid JSON");
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["allow-bootstrap-main"])
        );
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
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
