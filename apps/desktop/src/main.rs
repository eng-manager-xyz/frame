#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::sync::Mutex;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use frame_desktop_core::{
    DesktopAdapterKind, DesktopBootstrap, DesktopDispatch, DesktopRoots, DesktopRuntime,
    InstantFinalizeCommandV1, InstantFinalizeService, InstantFinalizeServiceError,
    InstantFinalizeUiUpdate, PublicErrorCode, ShellCapabilities,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::{Emitter, Manager};

#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAX_INSTANT_FINALIZE_COMMAND_BYTES: usize = 512;

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct NativeDesktopState {
    runtime: Mutex<DesktopRuntime>,
    instant_finalize: InstantFinalizeService,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
    state: tauri::State<'_, NativeDesktopState>,
) -> Result<ShellCapabilities, &'static str> {
    main_window(window.label())?;
    let capabilities = shell_capabilities(state.instant_finalize.capability());
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
        .runtime
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
        .runtime
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
            emit_instant_update(&app, &update)?;
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
    emit_instant_update(&app, &update)?;
    Ok(update)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn emit_instant_update(
    app: &tauri::AppHandle,
    update: &InstantFinalizeUiUpdate,
) -> Result<(), DesktopBoundaryError> {
    for event in &update.events {
        app.emit("frame-desktop://event-v1", event)
            .map_err(|_| DesktopBoundaryError {
                code: PublicErrorCode::Internal,
            })?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn shell_capabilities(
    instant_finalize: frame_desktop_core::InstantFinalizeCapabilityState,
) -> ShellCapabilities {
    ShellCapabilities {
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
            app.manage(NativeDesktopState {
                runtime: Mutex::new(runtime),
                instant_finalize: InstantFinalizeService::not_configured(),
            });
            Ok(())
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
                "allow-dispatch-main",
                "allow-finalize-instant"
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
        let capabilities =
            shell_capabilities(frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured);
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
        assert_eq!(
            capabilities.instant_finalize,
            frame_desktop_core::InstantFinalizeCapabilityState::NotConfigured
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
