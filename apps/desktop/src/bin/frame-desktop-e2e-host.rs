//! Deterministic native boundary used by the browser accessibility journey.
//!
//! This executable is never a production adapter. It is an explicitly gated
//! process that exposes the same serialized bootstrap and dispatch contracts
//! as the Tauri commands while retaining all state transitions in Rust.

use std::{
    env,
    ffi::OsString,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use frame_desktop_core::{
    DesktopAdapterKind, DesktopRoots, DesktopRuntime, EditorAdapterState, RecorderAdapterState,
    ShellCapabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_REQUEST_BYTES: usize = 70 * 1_024;

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum HostRequest {
    BootstrapMain,
    BootstrapDesktop,
    DispatchMain { request_json: String },
}

#[derive(Debug, Serialize)]
struct HostResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

impl HostResponse {
    fn success<T: Serialize>(value: T) -> Result<Self, serde_json::Error> {
        Ok(Self {
            ok: true,
            value: Some(serde_json::to_value(value)?),
            error: None,
        })
    }

    const fn failure(error: &'static str) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("frame desktop E2E host failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), &'static str> {
    if env::var("FRAME_DESKTOP_E2E").as_deref() != Ok("1") {
        return Err("FRAME_DESKTOP_E2E=1 is required");
    }
    let root = parse_root(env::args_os())?;
    let projects = root.join("projects");
    let media = root.join("media");
    let exports = root.join("exports");
    for path in [&projects, &media, &exports] {
        std::fs::create_dir_all(path).map_err(|_| "cannot create bounded E2E roots")?;
    }
    let roots = DesktopRoots::new(
        path_text(&projects)?,
        path_text(&media)?,
        path_text(&exports)?,
    );
    let mut runtime =
        DesktopRuntime::new(DesktopAdapterKind::DeterministicFake, roots, "browser-e2e")
            .map_err(|_| "cannot create deterministic desktop runtime")?;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    loop {
        let response = match read_request(&mut reader)? {
            RequestLine::Eof => break,
            RequestLine::Oversized => HostResponse::failure("request_too_large"),
            RequestLine::Value(line) => handle_request(&mut runtime, &line),
        };
        serde_json::to_writer(&mut writer, &response)
            .map_err(|_| "cannot serialize host response")?;
        writer
            .write_all(b"\n")
            .and_then(|()| writer.flush())
            .map_err(|_| "cannot write host response")?;
    }
    Ok(())
}

#[derive(Debug)]
enum RequestLine {
    Eof,
    Oversized,
    Value(String),
}

fn read_request(reader: &mut impl BufRead) -> Result<RequestLine, &'static str> {
    let mut encoded = Vec::with_capacity(1_024);
    let mut oversized = false;
    loop {
        let available = reader.fill_buf().map_err(|_| "cannot read host request")?;
        if available.is_empty() {
            if encoded.is_empty() && !oversized {
                return Ok(RequestLine::Eof);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !oversized {
            let payload_bytes = newline.unwrap_or(consumed);
            if encoded.len().saturating_add(payload_bytes) > MAX_REQUEST_BYTES {
                oversized = true;
                encoded.clear();
            } else {
                encoded.extend_from_slice(&available[..consumed]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if oversized {
        return Ok(RequestLine::Oversized);
    }
    String::from_utf8(encoded)
        .map(RequestLine::Value)
        .map_err(|_| "host request is not valid UTF-8")
}

fn handle_request(runtime: &mut DesktopRuntime, encoded: &str) -> HostResponse {
    let request: HostRequest = match serde_json::from_str(encoded) {
        Ok(request) => request,
        Err(_) => return HostResponse::failure("malformed_request"),
    };
    let response = match request {
        HostRequest::BootstrapMain => HostResponse::success(ShellCapabilities {
            recorder_adapter: RecorderAdapterState::DeterministicFake,
            editor_adapter: EditorAdapterState::RevisionFencedCore,
            ..ShellCapabilities::current()
        }),
        HostRequest::BootstrapDesktop => HostResponse::success(runtime.bootstrap()),
        HostRequest::DispatchMain { request_json } => {
            let mut dispatch = match runtime.dispatch_json(&request_json) {
                Ok(dispatch) => dispatch,
                Err(_) => return HostResponse::failure("dispatch_rejected"),
            };
            let background_running = matches!(
                dispatch.snapshot.export,
                frame_desktop_core::ExportState::Running { .. }
            ) || matches!(
                dispatch.snapshot.upload,
                frame_desktop_core::UploadState::Uploading { .. }
            );
            if background_running {
                let events = match runtime.advance_fake() {
                    Ok(events) => events,
                    Err(_) => return HostResponse::failure("background_advance_failed"),
                };
                dispatch.events.extend(events);
                dispatch.snapshot = runtime.snapshot();
            }
            HostResponse::success(dispatch)
        }
    };
    response.unwrap_or_else(|_| HostResponse::failure("serialization_failed"))
}

fn parse_root(mut arguments: impl Iterator<Item = OsString>) -> Result<PathBuf, &'static str> {
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--root")) {
        return Err("expected --root");
    }
    let root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("missing E2E root")?;
    if arguments.next().is_some() || !root.is_absolute() || root == Path::new("/") {
        return Err("E2E root must be one absolute non-root path");
    }
    Ok(root)
}

fn path_text(path: &Path) -> Result<String, &'static str> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or("E2E root is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{MAX_REQUEST_BYTES, RequestLine, read_request};

    #[test]
    fn bounded_reader_accepts_the_exact_payload_limit() {
        let mut input = vec![b'a'; MAX_REQUEST_BYTES];
        input.push(b'\n');
        let mut reader = Cursor::new(input);

        match read_request(&mut reader).expect("request must be readable") {
            RequestLine::Value(value) => {
                assert_eq!(value.len(), MAX_REQUEST_BYTES + 1);
                assert!(value.ends_with('\n'));
            }
            RequestLine::Eof | RequestLine::Oversized => panic!("exact limit was rejected"),
        }
    }

    #[test]
    fn bounded_reader_drains_an_oversized_line() {
        let mut input = vec![b'a'; MAX_REQUEST_BYTES + 1];
        input.extend_from_slice(b"\n{}\n");
        let mut reader = Cursor::new(input);

        assert!(matches!(
            read_request(&mut reader).expect("oversized request must be drained"),
            RequestLine::Oversized
        ));
        assert!(matches!(
            read_request(&mut reader).expect("next request must remain readable"),
            RequestLine::Value(value) if value == "{}\n"
        ));
    }

    #[test]
    fn bounded_reader_rejects_invalid_utf8() {
        let mut reader = Cursor::new([0xff, b'\n']);
        assert_eq!(
            read_request(&mut reader).expect_err("invalid UTF-8 must be rejected"),
            "host request is not valid UTF-8"
        );
    }
}
