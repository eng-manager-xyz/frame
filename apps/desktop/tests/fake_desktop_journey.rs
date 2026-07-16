use std::collections::HashMap;

use frame_desktop_core::{
    CaptureTargetKind, CommandOutcome, DesktopAdapterKind, DesktopRoots, DesktopRuntime,
    DeviceClass, EditorMutation, EditorState, ExportProfile, ExportState, IPC_PROTOCOL_VERSION,
    IpcCommand, RecorderState, RequestEnvelope, RequestId, UpdateAction, UpdateState, UploadState,
    WindowRole,
};

struct Harness {
    runtime: DesktopRuntime,
    sequence: HashMap<WindowRole, u64>,
    request: u64,
}

impl Harness {
    fn new() -> Self {
        Self {
            runtime: DesktopRuntime::new(
                DesktopAdapterKind::DeterministicFake,
                DesktopRoots::new("/frame/projects", "/frame/media", "/frame/exports"),
                "journey",
            )
            .expect("fake runtime"),
            sequence: HashMap::new(),
            request: 0,
        }
    }

    fn dispatch(&mut self, role: WindowRole, command: IpcCommand) {
        let context = self
            .runtime
            .bootstrap()
            .contexts
            .into_iter()
            .find(|context| context.role == role)
            .expect("window scope");
        let sequence = self.sequence.entry(role).or_insert(0);
        *sequence += 1;
        self.request += 1;
        let dispatch = self
            .runtime
            .dispatch(RequestEnvelope {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: RequestId::new(format!("journey-{:04}", self.request))
                    .expect("request ID"),
                window_id: context.window_id,
                session_id: context.session_id,
                sequence: *sequence,
                command,
            })
            .expect("accepted request");
        assert!(matches!(
            dispatch.response.outcome,
            CommandOutcome::Ok { .. }
        ));
        assert_eq!(dispatch.snapshot, self.runtime.snapshot());
        assert!(dispatch.events.iter().all(|event| event.owner == role));
    }
}

#[test]
fn keyboard_equivalent_fake_journey_reaches_every_essential_backend_state() {
    let mut harness = Harness::new();
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::DeviceEnumerate {
            class: DeviceClass::Display,
        },
    );
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::CaptureTargetSelect {
            kind: CaptureTargetKind::Display,
            target_token: "fake-display-1".into(),
        },
    );
    harness.dispatch(WindowRole::Recorder, IpcCommand::RecorderPrepare);
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::RecorderStart {
            intent_id: "record-start".into(),
        },
    );
    assert_eq!(
        harness.runtime.snapshot().recorder,
        RecorderState::Recording
    );
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::RecorderPause {
            intent_id: "record-pause".into(),
        },
    );
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::RecorderResume {
            intent_id: "record-resume".into(),
        },
    );
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::RecorderStop {
            intent_id: "record-stop".into(),
        },
    );
    assert_eq!(harness.runtime.snapshot().recorder, RecorderState::Ready);

    harness.dispatch(WindowRole::Recovery, IpcCommand::RecoveryScan);
    harness.dispatch(
        WindowRole::Recovery,
        IpcCommand::RecoveryOpen {
            project_path: "/frame/projects/demo.frame".into(),
        },
    );
    harness.dispatch(
        WindowRole::Editor,
        IpcCommand::EditorOpen {
            project_path: "/frame/projects/demo.frame".into(),
        },
    );
    harness.dispatch(
        WindowRole::Editor,
        IpcCommand::EditorApply {
            base_revision: 1,
            mutation: EditorMutation::Trim {
                start_ms: 1_000,
                end_ms: 80_000,
            },
        },
    );
    harness.dispatch(
        WindowRole::Editor,
        IpcCommand::EditorSave {
            expected_revision: 2,
        },
    );
    assert!(matches!(
        harness.runtime.snapshot().editor,
        EditorState::Ready {
            revision: 2,
            dirty: false,
            ..
        }
    ));

    harness.dispatch(
        WindowRole::Editor,
        IpcCommand::ExportStart {
            project_revision: 2,
            output_path: "/frame/exports/demo.mp4".into(),
            profile: ExportProfile::DistributionMp4,
        },
    );
    harness.dispatch(
        WindowRole::Editor,
        IpcCommand::UploadStart {
            source_path: "/frame/media/demo.mp4".into(),
            upload_intent: "upload-start".into(),
        },
    );
    harness.runtime.advance_fake().expect("background work");
    assert!(matches!(
        harness.runtime.snapshot().export,
        ExportState::Completed {
            project_revision: 2
        }
    ));
    assert_eq!(harness.runtime.snapshot().upload, UploadState::Completed);

    for action in [
        UpdateAction::Check,
        UpdateAction::Install,
        UpdateAction::Relaunch,
    ] {
        harness.dispatch(
            WindowRole::Main,
            IpcCommand::Update {
                action,
                expected_revision: 1,
            },
        );
    }
    assert_eq!(
        harness.runtime.snapshot().update,
        UpdateState::Current { revision: 2 }
    );
}

#[test]
fn fake_crash_and_device_loss_never_leave_the_ui_claiming_recording() {
    let mut harness = Harness::new();
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::DeviceEnumerate {
            class: DeviceClass::Display,
        },
    );
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::CaptureTargetSelect {
            kind: CaptureTargetKind::Display,
            target_token: "fake-display-1".into(),
        },
    );
    harness.dispatch(WindowRole::Recorder, IpcCommand::RecorderPrepare);
    harness.dispatch(
        WindowRole::Recorder,
        IpcCommand::RecorderStart {
            intent_id: "record-before-crash".into(),
        },
    );
    harness
        .runtime
        .simulate_fake_device_loss()
        .expect("device loss");
    assert_eq!(
        harness.runtime.snapshot().recorder,
        RecorderState::Recoverable
    );
    harness.runtime.simulate_fake_restart().expect("restart");
    let snapshot = harness.runtime.snapshot();
    assert!(snapshot.crash_recovery_reported);
    assert_ne!(snapshot.recorder, RecorderState::Recording);
    assert!(!snapshot.lifecycle.overlay_visible);
}
