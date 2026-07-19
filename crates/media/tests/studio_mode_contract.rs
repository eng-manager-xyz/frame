use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::Path,
    rc::Rc,
    time::Duration,
};

use frame_media::*;

fn project_id(marker: u8) -> StudioProjectId {
    StudioProjectId::from_csprng([marker; 16]).expect("project ID")
}

fn asset_id(marker: u8) -> StudioAssetId {
    StudioAssetId::from_csprng([marker; 16]).expect("asset ID")
}

fn operation_id(marker: u8) -> StudioOperationId {
    StudioOperationId::from_csprng([marker; 16]).expect("operation ID")
}

fn worker_id(marker: u8) -> StudioWorkerId {
    StudioWorkerId::from_csprng([marker; 16]).expect("worker ID")
}

fn export_id(marker: u8) -> StudioExportId {
    StudioExportId::from_csprng([marker; 16]).expect("export ID")
}

fn seconds(value: u64) -> RationalTime {
    RationalTime::new(value, TimeBase::new(1).expect("timebase"))
}

fn checksum(marker: u8) -> AssetChecksum {
    AssetChecksum::from_bytes([marker; 32]).expect("checksum")
}

fn recording_encoding(track: TrackKind) -> StudioAssetEncoding {
    match track {
        TrackKind::Screen | TrackKind::Camera => {
            StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
                width: 1_280,
                height: 720,
                frame_rate: FrameRate {
                    numerator: 30,
                    denominator: 1,
                },
                pixel_format: PixelFormat::Bgra8,
            })
            .expect("VP8/WebM recording encoding")
        }
        TrackKind::Microphone | TrackKind::SystemAudio => {
            StudioAssetEncoding::recording_opus_webm(StudioAudioRawCaps {
                sample_rate: 48_000,
                channels: 2,
                sample_format: AudioSampleFormat::Float32,
            })
            .expect("Opus/WebM recording encoding")
        }
    }
}

fn asset(marker: u8, track: TrackKind) -> StudioAsset {
    asset_with_range(marker, track, 0, 10)
}

fn asset_with_range(
    marker: u8,
    track: TrackKind,
    start_seconds: u64,
    duration_seconds: u64,
) -> StudioAsset {
    StudioAsset {
        version: STUDIO_ASSET_VERSION,
        id: asset_id(marker),
        track,
        source_name: StudioSourceName::new(format!("track-{marker}.webm")).expect("name"),
        byte_len: 4_096,
        start: seconds(start_seconds),
        duration: seconds(duration_seconds),
        checksum: checksum(marker),
        commit_state: AssetCommitState::DurableOriginal,
        encoding: recording_encoding(track),
    }
}

fn edit_spec() -> EditSpec {
    EditSpec {
        version: STUDIO_EDIT_VERSION,
        revision: 7,
        operations: vec![
            EditOperation::Trim {
                start: seconds(1),
                end: seconds(9),
            },
            EditOperation::Split { at: seconds(6) },
            EditOperation::Speed {
                start: seconds(2),
                end: seconds(4),
                numerator: 2,
                denominator: 1,
            },
            EditOperation::DeleteRange {
                start: seconds(4),
                end: seconds(5),
            },
            EditOperation::Layout {
                start: seconds(1),
                end: seconds(9),
                preset: LayoutPreset::SideBySide,
            },
            EditOperation::CameraTransform {
                start: seconds(1),
                end: seconds(9),
                rect: NormalizedRect {
                    x_millionths: 500_000,
                    y_millionths: 0,
                    width_millionths: 500_000,
                    height_millionths: 1_000_000,
                },
                corner_radius_milli: 0,
            },
            EditOperation::CursorTransform {
                start: seconds(1),
                end: seconds(9),
                scale_milli: 1_500,
                hidden: false,
            },
            EditOperation::Background {
                start: seconds(1),
                end: seconds(9),
                style: BackgroundStyle::SolidRgb {
                    red: 12,
                    green: 34,
                    blue: 56,
                },
            },
            EditOperation::AudioGain {
                track: TrackKind::Microphone,
                start: seconds(1),
                end: seconds(9),
                gain_millibels: -600,
                muted: false,
            },
            EditOperation::AudioGain {
                track: TrackKind::SystemAudio,
                start: seconds(1),
                end: seconds(9),
                gain_millibels: 0,
                muted: true,
            },
        ],
    }
}

fn source() -> TimelineSource {
    TimelineSource {
        duration: seconds(10),
        coverage: vec![
            SourceCoverage {
                track: TrackKind::Screen,
                start: seconds(0),
                end: seconds(10),
            },
            SourceCoverage {
                track: TrackKind::Camera,
                start: seconds(0),
                end: seconds(3),
            },
            SourceCoverage {
                track: TrackKind::Microphone,
                start: seconds(0),
                end: seconds(10),
            },
            SourceCoverage {
                track: TrackKind::SystemAudio,
                start: seconds(0),
                end: seconds(5),
            },
        ],
        vfr_video_pts: BTreeMap::from([(
            TrackKind::Screen,
            vec![
                RationalTime::new(0, TimeBase::new(2).expect("timebase")),
                RationalTime::new(1, TimeBase::new(2).expect("timebase")),
                RationalTime::new(3, TimeBase::new(2).expect("timebase")),
                RationalTime::new(5, TimeBase::new(2).expect("timebase")),
                RationalTime::new(9, TimeBase::new(2).expect("timebase")),
                RationalTime::new(11, TimeBase::new(2).expect("timebase")),
            ],
        )]),
    }
}

fn manifest() -> StudioProjectManifest {
    StudioProjectManifest {
        version: STUDIO_PROJECT_VERSION,
        id: project_id(1),
        revision: 9,
        state: StudioState::Editing,
        assets: vec![
            asset_with_range(2, TrackKind::Screen, 0, 10),
            asset_with_range(3, TrackKind::Camera, 0, 3),
            asset_with_range(4, TrackKind::Microphone, 0, 10),
            asset_with_range(5, TrackKind::SystemAudio, 0, 5),
        ],
        edits: edit_spec(),
    }
}

fn legacy_file(path: &str, bytes: &[u8]) -> LegacyCapFileDescriptor {
    LegacyCapFileDescriptor {
        relative_path: LegacyCapRelativePath::new(path).expect("legacy relative path"),
        byte_len: bytes.len() as u64,
        checksum: AssetChecksum::from_content(bytes),
    }
}

fn legacy_snapshot(
    version: u16,
    unsupported_effects: BTreeSet<LegacyUnsupportedEffect>,
) -> LegacyCapProjectSnapshot {
    let recording_meta =
        include_bytes!("../../../fixtures/studio/cap-schema-supported/recording-meta.json");
    let project_config =
        include_bytes!("../../../fixtures/studio/cap-schema-supported/project-config.json");
    LegacyCapProjectSnapshot::new(
        version,
        legacy_file("recording-meta.json", recording_meta),
        Some(legacy_file("project-config.json", project_config)),
        vec![
            LegacyCapSegment {
                index: 0,
                start: seconds(0),
                duration: seconds(5),
                display: legacy_file(
                    "content/segments/segment-0/display.mp4",
                    include_bytes!(
                        "../../../fixtures/studio/cap-schema-supported/content/segments/segment-0/display.mp4"
                    ),
                ),
                camera: Some(legacy_file(
                    "content/segments/segment-0/camera.mp4",
                    include_bytes!(
                        "../../../fixtures/studio/cap-schema-supported/content/segments/segment-0/camera.mp4"
                    ),
                )),
                microphone: Some(legacy_file(
                    "content/segments/segment-0/audio-input.ogg",
                    include_bytes!(
                        "../../../fixtures/studio/cap-schema-supported/content/segments/segment-0/audio-input.ogg"
                    ),
                )),
                system_audio: Some(legacy_file(
                    "content/segments/segment-0/system_audio.ogg",
                    include_bytes!(
                        "../../../fixtures/studio/cap-schema-supported/content/segments/segment-0/system_audio.ogg"
                    ),
                )),
            },
            LegacyCapSegment {
                index: 1,
                start: seconds(5),
                duration: seconds(5),
                display: legacy_file(
                    "content/segments/segment-1/display.mp4",
                    include_bytes!(
                        "../../../fixtures/studio/cap-schema-supported/content/segments/segment-1/display.mp4"
                    ),
                ),
                camera: None,
                microphone: None,
                system_audio: None,
            },
        ],
        EditSpec {
            version: STUDIO_EDIT_VERSION,
            revision: 1,
            operations: vec![
                EditOperation::Trim {
                    start: seconds(1),
                    end: seconds(9),
                },
                EditOperation::Split { at: seconds(6) },
            ],
        },
        unsupported_effects,
    )
    .expect("schema-faithful legacy snapshot")
}

#[derive(Debug)]
struct MemoryLegacyCapPort {
    snapshot: LegacyCapProjectSnapshot,
    fingerprint_calls: usize,
    mutate_after_read: bool,
}

impl LegacyCapProjectPort for MemoryLegacyCapPort {
    fn source_tree_digest(&mut self) -> Result<Sha256Digest, StudioError> {
        self.fingerprint_calls += 1;
        if self.mutate_after_read && self.fingerprint_calls > 1 {
            Ok(strong_sha256(b"legacy tree changed during inspection"))
        } else {
            Ok(self.snapshot.source_tree_digest)
        }
    }

    fn read_snapshot(&mut self) -> Result<LegacyCapProjectSnapshot, StudioError> {
        Ok(self.snapshot.clone())
    }
}

fn legacy_v1_asset_payload(marker: u8, track: TrackKind) -> Vec<u8> {
    let source_name = format!("track-{marker}.webm");
    let mut payload = Vec::new();
    payload.extend_from_slice(&1_u16.to_be_bytes());
    payload.extend_from_slice(&[marker; 16]);
    payload.push(match track {
        TrackKind::Screen => 1,
        TrackKind::Camera => 2,
        TrackKind::Microphone => 3,
        TrackKind::SystemAudio => 4,
    });
    payload.extend_from_slice(
        &u16::try_from(source_name.len())
            .expect("bounded source name")
            .to_be_bytes(),
    );
    payload.extend_from_slice(source_name.as_bytes());
    payload.extend_from_slice(&4_096_u64.to_be_bytes());
    payload.extend_from_slice(&0_u64.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&10_u64.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&[marker; 32]);
    payload.push(2);
    payload
}

fn legacy_document(kind: u8, version: u16, payload: &[u8]) -> Vec<u8> {
    let mut document = Vec::new();
    document.extend_from_slice(b"FRST");
    document.push(kind);
    document.extend_from_slice(&version.to_be_bytes());
    document.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("bounded fixture payload")
            .to_be_bytes(),
    );
    document.extend_from_slice(payload);
    let digest = strong_sha256(&document).to_hex();
    for pair in digest.as_bytes().chunks_exact(2) {
        document.push(
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex pair"), 16)
                .expect("digest byte"),
        );
    }
    document
}

fn legacy_v1_project_document() -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&STUDIO_PROJECT_VERSION.to_be_bytes());
    payload.extend_from_slice(&[1; 16]);
    payload.extend_from_slice(&1_u64.to_be_bytes());
    payload.push(4);
    payload.extend_from_slice(&1_u16.to_be_bytes());
    payload.extend_from_slice(&legacy_v1_asset_payload(2, TrackKind::Screen));
    payload.extend_from_slice(&STUDIO_EDIT_VERSION.to_be_bytes());
    payload.extend_from_slice(&1_u64.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    legacy_document(1, STUDIO_PROJECT_VERSION, &payload)
}

#[test]
fn v1_assets_migrate_to_an_explicit_unprobed_encoding() {
    let legacy_asset = legacy_document(4, 1, &legacy_v1_asset_payload(2, TrackKind::Screen));
    let migrated = StudioDocumentCodec::decode_asset(&legacy_asset).expect("migrate v1 asset");
    assert_eq!(migrated.version, STUDIO_ASSET_VERSION);
    assert_eq!(migrated.encoding, StudioAssetEncoding::UnspecifiedLegacyV1);
    assert!(migrated.requires_encoding_probe());

    let reencoded = StudioDocumentCodec::encode_asset(&migrated).expect("persist migration");
    assert_eq!(&reencoded[5..7], &STUDIO_ASSET_VERSION.to_be_bytes());
    assert_eq!(
        StudioDocumentCodec::decode_asset(&reencoded).expect("decode migrated asset"),
        migrated
    );

    let project = StudioDocumentCodec::decode_project(&legacy_v1_project_document())
        .expect("migrate embedded v1 asset");
    assert_eq!(
        project.assets[0].encoding,
        StudioAssetEncoding::UnspecifiedLegacyV1
    );
    assert!(project.assets[0].requires_encoding_probe());
    let resolved = project.assets[0]
        .with_probed_encoding(recording_encoding(TrackKind::Screen))
        .expect("persist a probed replacement descriptor");
    assert!(!resolved.requires_encoding_probe());
    assert_eq!(resolved.checksum, project.assets[0].checksum);
}

#[test]
fn canonical_documents_round_trip_and_fail_closed() {
    let project = manifest();
    let encoded_asset = StudioDocumentCodec::encode_asset(&project.assets[0]).expect("asset");
    assert_eq!(
        StudioDocumentCodec::decode_asset(&encoded_asset).expect("decode asset"),
        project.assets[0]
    );
    let encoded = StudioDocumentCodec::encode_project(&project).expect("encode project");
    assert_eq!(
        StudioDocumentCodec::encode_project(&project).expect("canonical encode"),
        encoded
    );
    assert_eq!(
        StudioDocumentCodec::decode_project(&encoded).expect("decode project"),
        project
    );

    let edit = StudioDocumentCodec::encode_edit(&project.edits).expect("encode edit");
    assert_eq!(
        StudioDocumentCodec::decode_edit(&edit).expect("decode edit"),
        project.edits
    );

    let mut corrupt = encoded.clone();
    let payload_byte = corrupt.get_mut(20).expect("payload byte");
    *payload_byte ^= 0x80;
    assert_eq!(
        StudioDocumentCodec::decode_project(&corrupt),
        Err(StudioError::CorruptDocument)
    );

    let mut newer = encoded.clone();
    newer[5..7].copy_from_slice(&2_u16.to_be_bytes());
    let payload_end = newer.len() - 32;
    let digest = strong_sha256(&newer[..payload_end]).to_hex();
    for (index, pair) in digest.as_bytes().chunks_exact(2).enumerate() {
        newer[payload_end + index] =
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex"), 16).expect("byte");
    }
    assert_eq!(
        StudioDocumentCodec::decode_project(&newer),
        Err(StudioError::UnsupportedProjectVersion(2))
    );

    let mut trailing = encoded;
    trailing.push(0);
    assert_eq!(
        StudioDocumentCodec::decode_project(&trailing),
        Err(StudioError::MalformedDocument)
    );
}

#[test]
fn legacy_cap_import_is_read_only_and_reports_unsupported_effects() {
    let supported = legacy_snapshot(LEGACY_CAP_SNAPSHOT_VERSION, BTreeSet::new());
    let assignment = LegacyIdAssignment {
        project_id: project_id(8),
        asset_ids: (9..14).map(asset_id).collect(),
    };
    let original = supported.clone();
    let mut port = MemoryLegacyCapPort {
        snapshot: supported,
        fingerprint_calls: 0,
        mutate_after_read: false,
    };
    let LegacyImportOutcome::Imported(imported) =
        import_legacy_cap(&mut port, &assignment).expect("import")
    else {
        panic!("supported fixture must import");
    };
    assert_eq!(port.snapshot, original);
    assert_eq!(imported.source_digest_before, imported.source_digest_after);
    assert!(imported.report.importable());
    assert_eq!(imported.manifest.state, StudioState::Editing);
    assert_eq!(imported.manifest.assets.len(), 5);
    assert_eq!(imported.manifest.assets[4].start, seconds(5));

    let mut unsupported = MemoryLegacyCapPort {
        snapshot: legacy_snapshot(
            LEGACY_CAP_SNAPSHOT_VERSION,
            BTreeSet::from([LegacyUnsupportedEffect::Zoom]),
        ),
        fingerprint_calls: 0,
        mutate_after_read: false,
    };
    let LegacyImportOutcome::NeedsUserAction(report) =
        import_legacy_cap(&mut unsupported, &assignment).expect("report")
    else {
        panic!("unsupported fixture must not import");
    };
    assert_eq!(
        report.unsupported_effects,
        BTreeSet::from([LegacyUnsupportedEffect::Zoom])
    );
    assert!(report.actionable_message.contains("legacy editor"));

    let mut newer = MemoryLegacyCapPort {
        snapshot: legacy_snapshot(LEGACY_CAP_SNAPSHOT_VERSION + 1, BTreeSet::new()),
        fingerprint_calls: 0,
        mutate_after_read: false,
    };
    assert!(matches!(
        import_legacy_cap(&mut newer, &assignment).expect("newer report"),
        LegacyImportOutcome::UnsupportedNewer(_)
    ));

    let mut changed = MemoryLegacyCapPort {
        snapshot: original,
        fingerprint_calls: 0,
        mutate_after_read: true,
    };
    assert_eq!(
        import_legacy_cap(&mut changed, &assignment),
        Err(StudioError::LegacySourceChanged)
    );
}

fn copy_fixture_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_fixture_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn supported_cap_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/studio/cap-schema-supported")
}

#[test]
fn filesystem_cap_adapter_parses_pinned_schema_and_emits_exact_copy_plan() {
    let fixture = supported_cap_fixture();
    let mut port = FilesystemLegacyCapProjectPort::open(&fixture).expect("filesystem Cap adapter");
    let parsed = port.read_snapshot().expect("parse pinned Cap schema");
    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[0].duration, seconds(5));
    assert_eq!(parsed.segments[1].start, seconds(5));
    assert_eq!(parsed.segments[1].duration, seconds(4));
    assert!(parsed.unsupported_effects.is_empty());
    assert_eq!(parsed.recording_meta.byte_len, 810);
    assert_eq!(
        port.source_tree_digest().expect("repeat fingerprint"),
        parsed.source_tree_digest
    );

    let assignment = LegacyIdAssignment {
        project_id: project_id(210),
        asset_ids: (211..=215).map(asset_id).collect(),
    };
    let LegacyImportOutcome::Imported(import) =
        import_legacy_cap(&mut port, &assignment).expect("filesystem import plan")
    else {
        panic!("supported real fixture must import");
    };
    assert_eq!(import.copy_plan.entries.len(), 5);
    assert_eq!(import.copy_plan.project_id, assignment.project_id);
    for (entry, asset) in import.copy_plan.entries.iter().zip(&import.manifest.assets) {
        assert_eq!(entry.asset_id, asset.id);
        assert_eq!(entry.track, asset.track);
        assert_eq!(entry.destination, asset.source_name);
        assert_eq!(entry.source.byte_len, asset.byte_len);
        assert_eq!(entry.source.checksum, asset.checksum);
    }
    assert_eq!(
        import.copy_plan.entries[0].source.relative_path.as_str(),
        "content/segments/segment-0/display.mp4"
    );
    assert_eq!(
        import.copy_plan.entries[4].source.relative_path.as_str(),
        "content/segments/segment-1/display.mp4"
    );
}

#[test]
fn filesystem_cap_adapter_accepts_pinned_single_segment_shape_and_known_defaults() {
    let fixture = supported_cap_fixture();
    let single = tempfile::tempdir().expect("single-segment directory");
    copy_fixture_tree(&fixture, single.path());
    let source_metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(single.path().join("recording-meta.json")).expect("read source metadata"),
    )
    .expect("parse source metadata");
    let first = source_metadata["segments"][0].clone();
    let single_metadata = serde_json::json!({
        "platform": "MacOS",
        "pretty_name": "Pinned single segment schema",
        "sharing": null,
        "display": first["display"].clone(),
        "camera": first["camera"].clone(),
        "audio": first["mic"].clone(),
        "cursor": null
    });
    fs::write(
        single.path().join("recording-meta.json"),
        serde_json::to_vec_pretty(&single_metadata).expect("encode single metadata"),
    )
    .expect("write single metadata");

    let config_path = single.path().join("project-config.json");
    let mut configuration: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read configuration"))
            .expect("parse configuration");
    configuration["timeline"]["segments"] =
        serde_json::Value::Array(vec![configuration["timeline"]["segments"][0].clone()]);
    for key in [
        "sceneSegments",
        "maskSegments",
        "textSegments",
        "captionSegments",
        "keyboardSegments",
        "audioSegments",
    ] {
        configuration["timeline"][key] = serde_json::json!([]);
    }
    configuration["audio"] = serde_json::json!({
        "mute": false,
        "improve": false,
        "micVolumeDb": 0.0,
        "micStereoMode": "stereo",
        "systemVolumeDb": 0.0
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&configuration).expect("encode known defaults"),
    )
    .expect("write known defaults");

    let mut port =
        FilesystemLegacyCapProjectPort::open(single.path()).expect("single-segment adapter");
    let snapshot = port.read_snapshot().expect("parse single-segment schema");
    assert_eq!(snapshot.segments.len(), 1);
    assert!(snapshot.unsupported_effects.is_empty());
    let assignment = LegacyIdAssignment {
        project_id: project_id(206),
        asset_ids: (207..=209).map(asset_id).collect(),
    };
    let LegacyImportOutcome::Imported(import) =
        import_legacy_cap(&mut port, &assignment).expect("single-segment import")
    else {
        panic!("known single-segment schema must import");
    };
    assert_eq!(import.copy_plan.entries.len(), 3);
    assert_eq!(import.manifest.assets.len(), 3);
}

#[test]
fn filesystem_cap_adapter_rejects_malformed_missing_and_adversarial_paths() {
    let fixture = supported_cap_fixture();

    let malformed = tempfile::tempdir().expect("malformed directory");
    copy_fixture_tree(&fixture, malformed.path());
    fs::write(malformed.path().join("recording-meta.json"), b"{not-json")
        .expect("write malformed metadata");
    let mut port =
        FilesystemLegacyCapProjectPort::open(malformed.path()).expect("open malformed fixture");
    assert_eq!(
        port.read_snapshot(),
        Err(StudioError::MalformedLegacyProject)
    );

    let traversal = tempfile::tempdir().expect("traversal directory");
    copy_fixture_tree(&fixture, traversal.path());
    let metadata_path = traversal.path().join("recording-meta.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata_path).expect("read metadata"))
            .expect("parse metadata for adversarial mutation");
    metadata["segments"][0]["display"]["path"] =
        serde_json::Value::String("../../outside.mp4".into());
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).expect("encode adversarial metadata"),
    )
    .expect("write traversal metadata");
    let mut port =
        FilesystemLegacyCapProjectPort::open(traversal.path()).expect("open traversal fixture");
    assert_eq!(
        port.read_snapshot(),
        Err(StudioError::MalformedLegacyProject)
    );

    let missing = tempfile::tempdir().expect("missing directory");
    copy_fixture_tree(&fixture, missing.path());
    fs::remove_file(
        missing
            .path()
            .join("content/segments/segment-1/display.mp4"),
    )
    .expect("remove referenced media");
    let mut port =
        FilesystemLegacyCapProjectPort::open(missing.path()).expect("open missing fixture");
    assert_eq!(
        port.read_snapshot(),
        Err(StudioError::MalformedLegacyProject)
    );

    #[cfg(unix)]
    {
        let linked = tempfile::tempdir().expect("symlink directory");
        copy_fixture_tree(&fixture, linked.path());
        let media = linked.path().join("content/segments/segment-1/display.mp4");
        fs::remove_file(&media).expect("remove media for symlink");
        std::os::unix::fs::symlink(
            fixture.join("content/segments/segment-1/display.mp4"),
            &media,
        )
        .expect("create adversarial symlink");
        let mut port =
            FilesystemLegacyCapProjectPort::open(linked.path()).expect("open linked fixture");
        assert_eq!(
            port.read_snapshot(),
            Err(StudioError::MalformedLegacyProject)
        );
    }
}

fn branch(marker: u8, track: TrackKind) -> IsolatedTrackBranch {
    IsolatedTrackBranch {
        track,
        asset_id: asset_id(marker),
        temporary_name: StudioSourceName::new(format!("temp-{marker}.webm")).expect("temp"),
        source: match track {
            TrackKind::Screen => CaptureElementFamily::NativeScreenBridge,
            TrackKind::Camera => CaptureElementFamily::NativeCameraBridge,
            TrackKind::Microphone => CaptureElementFamily::NativeMicrophoneBridge,
            TrackKind::SystemAudio => CaptureElementFamily::NativeSystemAudioBridge,
        },
        encoder: match track {
            TrackKind::Screen | TrackKind::Camera => CaptureElementFamily::Vp8Encoder,
            TrackKind::Microphone | TrackKind::SystemAudio => CaptureElementFamily::OpusEncoder,
        },
        muxer: CaptureElementFamily::WebMMux,
        encoding: recording_encoding(track),
        queue: BoundedMediaQueue {
            max_buffers: 64,
            max_bytes: 32 * 1024 * 1024,
            max_time_ns: 1_000_000_000,
        },
    }
}

#[test]
fn recording_graph_requires_screen_and_accepts_each_optional_track() {
    let branches = vec![
        branch(1, TrackKind::Screen),
        branch(2, TrackKind::Camera),
        branch(3, TrackKind::Microphone),
        branch(4, TrackKind::SystemAudio),
    ];
    StudioRecordingGraphSpec::new(project_id(1), operation_id(2), branches.clone())
        .expect("isolated graph");
    for optional in [
        None,
        Some(TrackKind::Camera),
        Some(TrackKind::Microphone),
        Some(TrackKind::SystemAudio),
    ] {
        let mut enabled = vec![branch(10, TrackKind::Screen)];
        if let Some(track) = optional {
            enabled.push(branch(11, track));
        }
        StudioRecordingGraphSpec::new(project_id(1), operation_id(2), enabled)
            .expect("screen plus independently optional track");
    }
    assert_eq!(
        StudioRecordingGraphSpec::new(
            project_id(1),
            operation_id(2),
            vec![branch(12, TrackKind::Microphone)],
        ),
        Err(StudioError::InvalidRecordingGraph)
    );
    assert_eq!(
        StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
            width: 1_280,
            height: 720,
            frame_rate: FrameRate {
                numerator: 30,
                denominator: 1,
            },
            pixel_format: PixelFormat::Nv12,
        }),
        Err(StudioError::InvalidAssetEncoding)
    );
    let mut mismatched_codec = branch(13, TrackKind::Screen);
    mismatched_codec.encoder = CaptureElementFamily::OpusEncoder;
    assert_eq!(
        StudioRecordingGraphSpec::new(project_id(1), operation_id(2), vec![mismatched_codec],),
        Err(StudioError::InvalidRecordingGraph)
    );

    let mut flattened = branches;
    flattened[3].track = TrackKind::Microphone;
    assert_eq!(
        StudioRecordingGraphSpec::new(project_id(1), operation_id(2), flattened),
        Err(StudioError::InvalidRecordingGraph)
    );
}

#[test]
fn v1_recording_graphs_fail_with_an_explicit_version_boundary() {
    let directory = tempfile::tempdir().expect("recording graph directory");
    let store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let project_marker = 14_u8;
    let clock_marker = 15_u8;
    let project_directory = directory
        .path()
        .join(format!("{project_marker:02x}").repeat(16));
    fs::create_dir_all(&project_directory).expect("project directory");
    fs::write(
        project_directory.join(format!(
            "{}.studio-recording-graph",
            format!("{clock_marker:02x}").repeat(16)
        )),
        legacy_document(6, 1, b"legacy graph identity"),
    )
    .expect("legacy graph fixture");
    let graph = StudioRecordingGraphSpec::new(
        project_id(project_marker),
        operation_id(clock_marker),
        vec![branch(16, TrackKind::Screen)],
    )
    .expect("current graph");
    assert!(matches!(
        FilesystemStudioRecordingSession::begin(&store, graph, 1_024),
        Err(StudioError::UnsupportedRecordingGraphVersion(1))
    ));
}

#[test]
fn filesystem_recording_session_seals_only_enabled_independent_originals() {
    let directory = tempfile::tempdir().expect("recording directory");
    let mut store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let branches = vec![
        branch(222, TrackKind::Screen),
        branch(224, TrackKind::Microphone),
    ];
    let graph = StudioRecordingGraphSpec::new(project_id(226), operation_id(227), branches)
        .expect("recording graph");
    let mut recording = FilesystemStudioRecordingSession::begin(&store, graph, 1_024)
        .expect("begin recording sinks");
    let payloads = [
        (TrackKind::Screen, b"screen-encoded".as_slice()),
        (TrackKind::Microphone, b"microphone-encoded".as_slice()),
    ];
    for (track, payload) in payloads {
        recording
            .write_encoded_chunk(track, payload)
            .expect("bounded encoded chunk");
    }
    let temporary = recording
        .finish(seconds(0), seconds(2))
        .expect("seal isolated temporary tracks");
    assert_eq!(temporary.len(), 2);
    assert!(
        temporary
            .iter()
            .all(|asset| !asset.requires_encoding_probe())
    );
    for (index, asset) in temporary.into_iter().enumerate() {
        let committed = commit_verified_temporary(
            &mut store,
            TempAssetCommitTicket::new(
                project_id(226),
                operation_id(228_u8.saturating_add(index as u8)),
                3,
                asset,
            )
            .expect("asset ticket"),
        )
        .expect("commit isolated original");
        assert_eq!(committed.commit_state, AssetCommitState::DurableOriginal);
        assert_eq!(
            store
                .probe_original(project_id(226), committed.id)
                .expect("probe committed original"),
            Some(committed)
        );
    }
}

#[test]
fn filesystem_recording_session_recovers_mixed_partial_and_sealed_tracks() {
    let directory = tempfile::tempdir().expect("recording recovery directory");
    let mut store = FilesystemStudioOriginalStore::new(directory.path()).expect("original store");
    let branches = vec![
        branch(201, TrackKind::Screen),
        branch(202, TrackKind::Camera),
        branch(203, TrackKind::Microphone),
        branch(204, TrackKind::SystemAudio),
    ];
    let graph = StudioRecordingGraphSpec::new(project_id(205), operation_id(206), branches)
        .expect("recording graph");
    let payloads = [
        (TrackKind::Screen, b"screen-before-crash".as_slice()),
        (TrackKind::Camera, b"camera-before-crash".as_slice()),
        (TrackKind::Microphone, b"microphone-before-crash".as_slice()),
        (
            TrackKind::SystemAudio,
            b"system-audio-before-crash".as_slice(),
        ),
    ];
    let mut recording = FilesystemStudioRecordingSession::begin(&store, graph.clone(), 1_024)
        .expect("begin recording sinks");
    for (track, payload) in payloads {
        recording
            .write_encoded_chunk(track, payload)
            .expect("write pre-crash chunk");
    }
    drop(recording);
    assert!(matches!(
        FilesystemStudioRecordingSession::recover(&store, graph.clone(), 2_048),
        Err(StudioError::JournalCorrupt)
    ));

    let project_stem = format!("{:02x}", 205).repeat(16);
    let screen_stem = format!("{:02x}", 201).repeat(16);
    let project_directory = directory.path().join(project_stem);
    fs::rename(
        project_directory
            .join("recording-partials")
            .join(format!("{screen_stem}.recording-partial")),
        project_directory
            .join("temporary")
            .join(format!("{screen_stem}.media")),
    )
    .expect("simulate crash after sealing one track");

    let mut recovered =
        FilesystemStudioRecordingSession::recover(&store, graph, 1_024).expect("reopen sinks");
    assert_eq!(
        recovered.write_encoded_chunk(TrackKind::Screen, b"late-screen"),
        Err(StudioError::InvalidRecordingGraph),
        "an already-sealed track must remain immutable"
    );
    for track in [
        TrackKind::Camera,
        TrackKind::Microphone,
        TrackKind::SystemAudio,
    ] {
        recovered
            .write_encoded_chunk(track, b"-after-restart")
            .expect("resume unsealed track");
    }
    let temporary = recovered
        .finish(seconds(0), seconds(2))
        .expect("finish mixed recovered state");
    assert_eq!(temporary.len(), 4);
    assert_eq!(
        temporary
            .iter()
            .find(|asset| asset.track == TrackKind::Screen)
            .expect("screen asset")
            .checksum,
        AssetChecksum::from_content(b"screen-before-crash")
    );
    for (index, asset) in temporary.into_iter().enumerate() {
        commit_verified_temporary(
            &mut store,
            TempAssetCommitTicket::new(
                project_id(205),
                operation_id(207_u8.saturating_add(index as u8)),
                4,
                asset,
            )
            .expect("asset ticket"),
        )
        .expect("commit recovered original");
    }
}

#[derive(Debug, Default)]
struct JournalState {
    snapshot: Option<StudioJournalSnapshot>,
    lose_next_ack: bool,
}

#[derive(Debug, Clone, Default)]
struct FakeJournal {
    state: Rc<RefCell<JournalState>>,
}

impl FakeJournal {
    fn lose_next_ack(&self) {
        self.state.borrow_mut().lose_next_ack = true;
    }
}

impl StudioJournalPort for FakeJournal {
    fn load(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioJournalSnapshot>, StudioError> {
        Ok(self
            .state
            .borrow()
            .snapshot
            .clone()
            .filter(|snapshot| snapshot.project_id == project_id))
    }

    fn create(
        &mut self,
        initial: StudioJournalSnapshot,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError> {
        let mut state = self.state.borrow_mut();
        if let Some(existing) = state.snapshot.clone() {
            return Ok(StudioPortOutcome::Conflict(Box::new(existing)));
        }
        state.snapshot = Some(initial.clone());
        if std::mem::take(&mut state.lose_next_ack) {
            Ok(StudioPortOutcome::AcknowledgementLost)
        } else {
            Ok(StudioPortOutcome::Committed(initial))
        }
    }

    fn compare_and_swap(
        &mut self,
        request: StudioJournalCasRequest,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError> {
        let mut state = self.state.borrow_mut();
        let current = state.snapshot.clone().ok_or(StudioError::JournalCorrupt)?;
        if current.project_id != request.project_id
            || current.revision != request.expected_revision
            || current.fence != request.expected_fence
        {
            return Ok(StudioPortOutcome::Conflict(Box::new(current)));
        }
        state.snapshot = Some(request.next.clone());
        if std::mem::take(&mut state.lose_next_ack) {
            Ok(StudioPortOutcome::AcknowledgementLost)
        } else {
            Ok(StudioPortOutcome::Committed(request.next))
        }
    }
}

fn initial_journal() -> StudioJournalSnapshot {
    StudioJournalSnapshot {
        version: STUDIO_JOURNAL_VERSION,
        project_id: project_id(20),
        revision: 1,
        fence: 1,
        owner: worker_id(21),
        boundary: JournalBoundary::Created,
        last_operation_id: None,
        pending_asset: None,
        pending_edit: None,
        pending_render: None,
        receipts: BTreeMap::new(),
    }
}

#[test]
fn journal_cas_reconciles_lost_ack_and_fences_stale_writers() {
    let port = FakeJournal::default();
    let control = port.clone();
    let mut journal = DurableStudioJournal::create(port, initial_journal()).expect("journal");
    control.lose_next_ack();
    let command = strong_sha256(b"prepare graph");
    let outcome = strong_sha256(b"graph prepared");
    assert_eq!(
        journal
            .advance(JournalAdvanceRequest {
                expected_revision: 1,
                expected_fence: 1,
                operation_id: operation_id(22),
                command_digest: command,
                boundary: JournalBoundary::RecordingGraphPrepared,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipt_kind: ReceiptKind::GraphPrepared,
                outcome_digest: outcome,
            })
            .expect("lost ack reconciled"),
        JournalCommitOutcome::ReconciledAfterLostAcknowledgement
    );
    assert_eq!(
        journal
            .advance(JournalAdvanceRequest {
                expected_revision: 2,
                expected_fence: 1,
                operation_id: operation_id(22),
                command_digest: command,
                boundary: JournalBoundary::RecordingGraphPrepared,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipt_kind: ReceiptKind::GraphPrepared,
                outcome_digest: outcome,
            })
            .expect("replay"),
        JournalCommitOutcome::IdempotentReplay
    );
    assert_eq!(
        journal.advance(JournalAdvanceRequest {
            expected_revision: 1,
            expected_fence: 1,
            operation_id: operation_id(23),
            command_digest: strong_sha256(b"stale"),
            boundary: JournalBoundary::CaptureStarted,
            pending_asset: None,
            pending_edit: None,
            pending_render: None,
            receipt_kind: ReceiptKind::CaptureStarted,
            outcome_digest: strong_sha256(b"stale outcome"),
        }),
        Err(StudioError::StaleJournal)
    );
    journal
        .take_ownership(2, 1, worker_id(24))
        .expect("new owner");
    assert_eq!(journal.snapshot().fence, 2);

    let bytes = StudioDocumentCodec::encode_journal(journal.snapshot()).expect("encode journal");
    assert_eq!(
        StudioDocumentCodec::decode_journal(&bytes).expect("decode journal"),
        *journal.snapshot()
    );
}

#[test]
fn concurrent_edit_saves_serialize_by_revision_and_fence() {
    let port = FakeJournal::default();
    let mut first = DurableStudioJournal::create(port.clone(), initial_journal()).expect("first");
    let mut second = DurableStudioJournal::create(port, initial_journal()).expect("second");
    let request = |marker| JournalAdvanceRequest {
        expected_revision: 1,
        expected_fence: 1,
        operation_id: operation_id(marker),
        command_digest: strong_sha256(&[marker]),
        boundary: JournalBoundary::RecordingGraphPrepared,
        pending_asset: None,
        pending_edit: None,
        pending_render: None,
        receipt_kind: ReceiptKind::GraphPrepared,
        outcome_digest: strong_sha256(&[marker, marker]),
    };
    assert_eq!(
        first.advance(request(40)).expect("first writer"),
        JournalCommitOutcome::Committed
    );
    assert_eq!(second.advance(request(41)), Err(StudioError::StaleJournal));
    assert_eq!(second.snapshot().revision, 2);
}

#[test]
fn every_power_loss_boundary_has_a_safe_recovery_action() {
    let cases = [
        (
            JournalBoundary::Created,
            StudioRecoveryDirective::DiscardUnstartedTemporaryFiles,
        ),
        (
            JournalBoundary::RecordingGraphPrepared,
            StudioRecoveryDirective::DiscardUnstartedTemporaryFiles,
        ),
        (
            JournalBoundary::CaptureStarted,
            StudioRecoveryDirective::ResumeOrSealIsolatedTracks,
        ),
        (
            JournalBoundary::TempAssetReserved,
            StudioRecoveryDirective::DeleteUncommittedTemporaryAsset,
        ),
        (
            JournalBoundary::TempAssetDurable,
            StudioRecoveryDirective::ProbeAndCommitExactTemporaryAsset,
        ),
        (
            JournalBoundary::AssetCommitRequested,
            StudioRecoveryDirective::ProbeAndCommitExactTemporaryAsset,
        ),
        (
            JournalBoundary::AssetCommitted,
            StudioRecoveryDirective::ContinueRecording,
        ),
        (
            JournalBoundary::RecordingStopped,
            StudioRecoveryDirective::OpenEditor,
        ),
        (
            JournalBoundary::EditSavePrepared,
            StudioRecoveryDirective::ReconcileEditSaveByDigest,
        ),
        (
            JournalBoundary::EditSaveCommitted,
            StudioRecoveryDirective::OpenEditor,
        ),
        (
            JournalBoundary::RenderPrepared,
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor,
        ),
        (
            JournalBoundary::RenderRunning,
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor,
        ),
        (
            JournalBoundary::RenderFinalizing,
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor,
        ),
        (
            JournalBoundary::RenderCommitted,
            StudioRecoveryDirective::VerifyCommittedRenderThenOpenEditor,
        ),
        (
            JournalBoundary::RenderCancelled,
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor,
        ),
        (
            JournalBoundary::FailedRecoverably,
            StudioRecoveryDirective::RequireOperatorDecision,
        ),
    ];
    for (boundary, expected) in cases {
        assert_eq!(recovery_directive(boundary), expected, "{boundary:?}");
        let boundary_operation = operation_id(75);
        let pending_asset = if matches!(
            boundary,
            JournalBoundary::TempAssetReserved
                | JournalBoundary::TempAssetDurable
                | JournalBoundary::AssetCommitRequested
        ) {
            let mut temporary = asset(70, TrackKind::Screen);
            temporary.commit_state = AssetCommitState::Temporary;
            Some(PendingAssetCommit {
                operation_id: boundary_operation,
                asset: temporary,
            })
        } else if boundary == JournalBoundary::AssetCommitted {
            Some(PendingAssetCommit {
                operation_id: boundary_operation,
                asset: asset(70, TrackKind::Screen),
            })
        } else {
            None
        };
        let edit_boundary = matches!(
            boundary,
            JournalBoundary::EditSavePrepared | JournalBoundary::EditSaveCommitted
        );
        let edit_operation = if boundary == JournalBoundary::EditSavePrepared {
            boundary_operation
        } else {
            operation_id(78)
        };
        let pending_edit = edit_boundary.then(|| PendingEditSave {
            operation_id: edit_operation,
            expected_project_revision: 6,
            edits: edit_spec(),
        });
        let render_boundary = matches!(
            boundary,
            JournalBoundary::RenderPrepared
                | JournalBoundary::RenderRunning
                | JournalBoundary::RenderFinalizing
                | JournalBoundary::RenderCommitted
                | JournalBoundary::RenderCancelled
        );
        let render_operation = if boundary == JournalBoundary::RenderPrepared {
            boundary_operation
        } else {
            operation_id(76)
        };
        let pending_render = render_boundary.then(|| {
            let mut pending = PendingRender::new(
                render_operation,
                export_id(77),
                3,
                strong_sha256(b"recovery source set"),
                strong_sha256(b"canonical edit plan"),
                ExportProfile::DistributionMaster,
                StudioSourceName::new("recovery-export.mp4").expect("output"),
            )
            .expect("pending render");
            if boundary == JournalBoundary::RenderCommitted {
                pending.terminal_receipt = Some(RenderReceipt {
                    project_id: project_id(73),
                    export_id: pending.export_id,
                    operation_id: pending.operation_id,
                    fence: pending.fence,
                    source_set_digest: pending.source_set_digest,
                    plan_digest: pending.plan_digest,
                    render_spec_digest: pending.render_spec_digest,
                    profile: ExportProfileSpec::approved(pending.profile),
                    output_name: pending.output_name.clone(),
                    output_checksum: checksum(77),
                    output_bytes: 4_096,
                });
            }
            pending
        });
        let receipt_kind = match boundary {
            JournalBoundary::Created => None,
            JournalBoundary::RecordingGraphPrepared => Some(ReceiptKind::GraphPrepared),
            JournalBoundary::CaptureStarted => Some(ReceiptKind::CaptureStarted),
            JournalBoundary::TempAssetReserved => Some(ReceiptKind::TempReserved),
            JournalBoundary::TempAssetDurable => Some(ReceiptKind::TempDurable),
            JournalBoundary::AssetCommitRequested => Some(ReceiptKind::AssetCommitRequested),
            JournalBoundary::AssetCommitted => Some(ReceiptKind::AssetCommitted),
            JournalBoundary::RecordingStopped => Some(ReceiptKind::RecordingStopped),
            JournalBoundary::EditSavePrepared => Some(ReceiptKind::EditPrepared),
            JournalBoundary::EditSaveCommitted => Some(ReceiptKind::EditCommitted),
            JournalBoundary::RenderPrepared => Some(ReceiptKind::RenderPrepared),
            JournalBoundary::RenderRunning => Some(ReceiptKind::RenderStarted),
            JournalBoundary::RenderFinalizing => Some(ReceiptKind::RenderFinalizing),
            JournalBoundary::RenderCommitted => Some(ReceiptKind::RenderCommitted),
            JournalBoundary::RenderCancelled => Some(ReceiptKind::PartialDeleted),
            JournalBoundary::FailedRecoverably => Some(ReceiptKind::RecoveryApplied),
        };
        let mut receipts = receipt_kind.map_or_else(BTreeMap::new, |kind| {
            BTreeMap::from([(
                boundary_operation,
                StudioOperationReceipt {
                    operation_id: boundary_operation,
                    kind,
                    command_digest: strong_sha256(b"boundary command"),
                    outcome_digest: strong_sha256(b"boundary outcome"),
                },
            )])
        });
        if render_boundary && render_operation != boundary_operation {
            receipts.insert(
                render_operation,
                StudioOperationReceipt {
                    operation_id: render_operation,
                    kind: ReceiptKind::RenderPrepared,
                    command_digest: strong_sha256(b"render prepared command"),
                    outcome_digest: strong_sha256(b"render prepared outcome"),
                },
            );
        }
        if edit_boundary && edit_operation != boundary_operation {
            receipts.insert(
                edit_operation,
                StudioOperationReceipt {
                    operation_id: edit_operation,
                    kind: ReceiptKind::EditPrepared,
                    command_digest: strong_sha256(b"edit prepared command"),
                    outcome_digest: strong_sha256(b"edit prepared outcome"),
                },
            );
        }
        let snapshot = StudioJournalSnapshot {
            version: STUDIO_JOURNAL_VERSION,
            project_id: project_id(73),
            revision: 8,
            fence: 3,
            owner: worker_id(74),
            boundary,
            last_operation_id: receipt_kind.map(|_| boundary_operation),
            pending_asset,
            pending_edit,
            pending_render,
            receipts,
        };
        let encoded = StudioDocumentCodec::encode_journal(&snapshot).expect("power-loss snapshot");
        assert_eq!(
            StudioDocumentCodec::decode_journal(&encoded).expect("recover snapshot"),
            snapshot
        );
    }
}

fn journal_request(
    journal: &DurableStudioJournal<FakeJournal>,
    marker: u8,
    boundary: JournalBoundary,
    receipt_kind: ReceiptKind,
    pending_asset: Option<PendingAssetCommit>,
    pending_edit: Option<PendingEditSave>,
    pending_render: Option<PendingRender>,
) -> JournalAdvanceRequest {
    JournalAdvanceRequest {
        expected_revision: journal.snapshot().revision,
        expected_fence: journal.snapshot().fence,
        operation_id: operation_id(marker),
        command_digest: strong_sha256(&[marker, 1]),
        boundary,
        pending_asset,
        pending_edit,
        pending_render,
        receipt_kind,
        outcome_digest: strong_sha256(&[marker, 2]),
    }
}

fn recording_stopped_journal(
    graph_marker: u8,
    capture_marker: u8,
    stopped_marker: u8,
) -> DurableStudioJournal<FakeJournal> {
    let mut journal =
        DurableStudioJournal::create(FakeJournal::default(), initial_journal()).expect("journal");
    for (marker, boundary, receipt_kind) in [
        (
            graph_marker,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
        ),
        (
            capture_marker,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
        ),
        (
            stopped_marker,
            JournalBoundary::RecordingStopped,
            ReceiptKind::RecordingStopped,
        ),
    ] {
        journal
            .advance(journal_request(
                &journal,
                marker,
                boundary,
                receipt_kind,
                None,
                None,
                None,
            ))
            .expect("recording transition");
    }
    journal
}

#[test]
fn failed_recovery_exits_preserve_each_pending_identity_until_exact_resolution() {
    let mut asset_journal =
        DurableStudioJournal::create(FakeJournal::default(), initial_journal()).expect("journal");
    for (marker, boundary, receipt_kind) in [
        (
            160,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
        ),
        (
            161,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
        ),
    ] {
        asset_journal
            .advance(journal_request(
                &asset_journal,
                marker,
                boundary,
                receipt_kind,
                None,
                None,
                None,
            ))
            .expect("capture transition");
    }
    let mut temporary = asset_with_range(162, TrackKind::Screen, 0, 5);
    temporary.commit_state = AssetCommitState::Temporary;
    let pending_asset = PendingAssetCommit {
        operation_id: operation_id(162),
        asset: temporary.clone(),
    };
    asset_journal
        .advance(journal_request(
            &asset_journal,
            162,
            JournalBoundary::TempAssetReserved,
            ReceiptKind::TempReserved,
            Some(pending_asset.clone()),
            None,
            None,
        ))
        .expect("asset reserved");
    asset_journal
        .advance(journal_request(
            &asset_journal,
            163,
            JournalBoundary::FailedRecoverably,
            ReceiptKind::RecoveryApplied,
            Some(pending_asset.clone()),
            None,
            None,
        ))
        .expect("asset failure");
    assert_eq!(
        asset_journal.advance(journal_request(
            &asset_journal,
            164,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
            None,
            None,
            None,
        )),
        Err(StudioError::JournalPendingIdentityChanged)
    );
    let mut durable = temporary;
    durable.commit_state = AssetCommitState::DurableOriginal;
    asset_journal
        .advance(journal_request(
            &asset_journal,
            165,
            JournalBoundary::AssetCommitted,
            ReceiptKind::AssetCommitted,
            Some(PendingAssetCommit {
                operation_id: operation_id(165),
                asset: durable,
            }),
            None,
            None,
        ))
        .expect("exact asset recovery");

    let mut edit_journal = recording_stopped_journal(166, 167, 168);
    let pending_edit = PendingEditSave {
        operation_id: operation_id(169),
        expected_project_revision: 6,
        edits: edit_spec(),
    };
    edit_journal
        .advance(journal_request(
            &edit_journal,
            169,
            JournalBoundary::EditSavePrepared,
            ReceiptKind::EditPrepared,
            None,
            Some(pending_edit.clone()),
            None,
        ))
        .expect("edit prepared");
    edit_journal
        .advance(journal_request(
            &edit_journal,
            170,
            JournalBoundary::FailedRecoverably,
            ReceiptKind::RecoveryApplied,
            None,
            Some(pending_edit.clone()),
            None,
        ))
        .expect("edit failure");
    assert_eq!(
        edit_journal.advance(journal_request(
            &edit_journal,
            171,
            JournalBoundary::EditSaveCommitted,
            ReceiptKind::EditCommitted,
            None,
            None,
            None,
        )),
        Err(StudioError::JournalPendingIdentityChanged)
    );
    edit_journal
        .advance(journal_request(
            &edit_journal,
            171,
            JournalBoundary::EditSaveCommitted,
            ReceiptKind::EditCommitted,
            None,
            Some(pending_edit),
            None,
        ))
        .expect("exact edit recovery");

    let mut render_journal = recording_stopped_journal(172, 173, 174);
    let pending_render = PendingRender::new(
        operation_id(175),
        export_id(176),
        1,
        strong_sha256(b"failed recovery sources"),
        strong_sha256(b"failed recovery plan"),
        ExportProfile::DistributionMaster,
        StudioSourceName::new("failed-recovery.mp4").expect("output"),
    )
    .expect("pending render");
    render_journal
        .advance(journal_request(
            &render_journal,
            175,
            JournalBoundary::RenderPrepared,
            ReceiptKind::RenderPrepared,
            None,
            None,
            Some(pending_render.clone()),
        ))
        .expect("render prepared");
    render_journal
        .advance(journal_request(
            &render_journal,
            177,
            JournalBoundary::FailedRecoverably,
            ReceiptKind::RecoveryApplied,
            None,
            None,
            Some(pending_render.clone()),
        ))
        .expect("render failure");
    assert_eq!(
        render_journal.advance(journal_request(
            &render_journal,
            178,
            JournalBoundary::RenderCancelled,
            ReceiptKind::PartialDeleted,
            None,
            None,
            None,
        )),
        Err(StudioError::JournalPendingIdentityChanged)
    );
    render_journal
        .advance(journal_request(
            &render_journal,
            178,
            JournalBoundary::RenderCancelled,
            ReceiptKind::PartialDeleted,
            None,
            None,
            Some(pending_render),
        ))
        .expect("exact render recovery");
}

#[test]
fn journal_transitions_preserve_exact_asset_and_render_identity() {
    let mut journal =
        DurableStudioJournal::create(FakeJournal::default(), initial_journal()).expect("journal");
    journal
        .advance(journal_request(
            &journal,
            101,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
            None,
            None,
            None,
        ))
        .expect("graph prepared");
    journal
        .advance(journal_request(
            &journal,
            102,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
            None,
            None,
            None,
        ))
        .expect("capture started");
    let mut temporary = asset_with_range(160, TrackKind::Screen, 0, 5);
    temporary.commit_state = AssetCommitState::Temporary;
    journal
        .advance(journal_request(
            &journal,
            103,
            JournalBoundary::TempAssetReserved,
            ReceiptKind::TempReserved,
            Some(PendingAssetCommit {
                operation_id: operation_id(103),
                asset: temporary.clone(),
            }),
            None,
            None,
        ))
        .expect("temporary reserved");

    let mut mutations = Vec::new();
    let mut changed_id = temporary.clone();
    changed_id.id = asset_id(161);
    mutations.push(changed_id);
    let mut changed_name = temporary.clone();
    changed_name.source_name = StudioSourceName::new("changed-name.mkv").expect("name");
    mutations.push(changed_name);
    let mut changed_checksum = temporary.clone();
    changed_checksum.checksum = checksum(162);
    mutations.push(changed_checksum);
    let mut changed_timing = temporary.clone();
    changed_timing.duration = seconds(4);
    mutations.push(changed_timing);
    for changed in mutations {
        assert_eq!(
            journal.advance(journal_request(
                &journal,
                104,
                JournalBoundary::TempAssetDurable,
                ReceiptKind::TempDurable,
                Some(PendingAssetCommit {
                    operation_id: operation_id(104),
                    asset: changed,
                }),
                None,
                None,
            )),
            Err(StudioError::JournalPendingIdentityChanged)
        );
    }
    journal
        .advance(journal_request(
            &journal,
            104,
            JournalBoundary::TempAssetDurable,
            ReceiptKind::TempDurable,
            Some(PendingAssetCommit {
                operation_id: operation_id(104),
                asset: temporary.clone(),
            }),
            None,
            None,
        ))
        .expect("temporary durable");
    journal
        .advance(journal_request(
            &journal,
            105,
            JournalBoundary::AssetCommitRequested,
            ReceiptKind::AssetCommitRequested,
            Some(PendingAssetCommit {
                operation_id: operation_id(105),
                asset: temporary.clone(),
            }),
            None,
            None,
        ))
        .expect("commit requested");
    let mut durable = temporary;
    durable.commit_state = AssetCommitState::DurableOriginal;
    journal
        .advance(journal_request(
            &journal,
            106,
            JournalBoundary::AssetCommitted,
            ReceiptKind::AssetCommitted,
            Some(PendingAssetCommit {
                operation_id: operation_id(106),
                asset: durable,
            }),
            None,
            None,
        ))
        .expect("asset committed");

    let mut render_journal =
        DurableStudioJournal::create(FakeJournal::default(), initial_journal()).expect("journal");
    for (marker, boundary, kind) in [
        (
            111,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
        ),
        (
            112,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
        ),
        (
            113,
            JournalBoundary::RecordingStopped,
            ReceiptKind::RecordingStopped,
        ),
    ] {
        render_journal
            .advance(journal_request(
                &render_journal,
                marker,
                boundary,
                kind,
                None,
                None,
                None,
            ))
            .expect("recording boundary");
    }
    let pending = PendingRender::new(
        operation_id(114),
        export_id(115),
        1,
        strong_sha256(b"source set identity"),
        strong_sha256(b"edit plan identity"),
        ExportProfile::DistributionMaster,
        StudioSourceName::new("journal-export.mp4").expect("output"),
    )
    .expect("pending render");
    render_journal
        .advance(journal_request(
            &render_journal,
            114,
            JournalBoundary::RenderPrepared,
            ReceiptKind::RenderPrepared,
            None,
            None,
            Some(pending.clone()),
        ))
        .expect("render prepared");

    let mutated_pending = [
        PendingRender::new(
            pending.operation_id,
            export_id(116),
            pending.fence,
            pending.source_set_digest,
            pending.plan_digest,
            pending.profile,
            pending.output_name.clone(),
        )
        .expect("changed export"),
        PendingRender::new(
            pending.operation_id,
            pending.export_id,
            pending.fence,
            strong_sha256(b"changed source set"),
            pending.plan_digest,
            pending.profile,
            pending.output_name.clone(),
        )
        .expect("changed source"),
        PendingRender::new(
            pending.operation_id,
            pending.export_id,
            pending.fence,
            pending.source_set_digest,
            strong_sha256(b"changed plan"),
            pending.profile,
            pending.output_name.clone(),
        )
        .expect("changed plan"),
        PendingRender::new(
            pending.operation_id,
            pending.export_id,
            pending.fence,
            pending.source_set_digest,
            pending.plan_digest,
            pending.profile,
            StudioSourceName::new("changed-output.mp4").expect("output"),
        )
        .expect("changed output"),
    ];
    for changed in mutated_pending {
        assert_eq!(
            render_journal.advance(journal_request(
                &render_journal,
                117,
                JournalBoundary::RenderRunning,
                ReceiptKind::RenderStarted,
                None,
                None,
                Some(changed),
            )),
            Err(StudioError::JournalPendingIdentityChanged)
        );
    }
    render_journal
        .advance(journal_request(
            &render_journal,
            117,
            JournalBoundary::RenderRunning,
            ReceiptKind::RenderStarted,
            None,
            None,
            Some(pending.clone()),
        ))
        .expect("render running");
    render_journal
        .advance(journal_request(
            &render_journal,
            118,
            JournalBoundary::RenderFinalizing,
            ReceiptKind::RenderFinalizing,
            None,
            None,
            Some(pending.clone()),
        ))
        .expect("render finalizing");
    let mut committed_pending = pending;
    committed_pending.terminal_receipt = Some(RenderReceipt {
        project_id: render_journal.snapshot().project_id,
        export_id: committed_pending.export_id,
        operation_id: committed_pending.operation_id,
        fence: committed_pending.fence,
        source_set_digest: committed_pending.source_set_digest,
        plan_digest: committed_pending.plan_digest,
        render_spec_digest: committed_pending.render_spec_digest,
        profile: ExportProfileSpec::approved(committed_pending.profile),
        output_name: committed_pending.output_name.clone(),
        output_checksum: checksum(119),
        output_bytes: 8_192,
    });
    render_journal
        .advance(journal_request(
            &render_journal,
            119,
            JournalBoundary::RenderCommitted,
            ReceiptKind::RenderCommitted,
            None,
            None,
            Some(committed_pending),
        ))
        .expect("render committed");
    let encoded =
        StudioDocumentCodec::encode_journal(render_journal.snapshot()).expect("journal encode");
    assert_eq!(
        StudioDocumentCodec::decode_journal(&encoded).expect("journal reopen"),
        *render_journal.snapshot()
    );
}

#[derive(Debug, Default)]
struct FakeOriginalStore {
    committed: Option<StudioAsset>,
    acknowledgement_lost: bool,
}

impl StudioOriginalStorePort for FakeOriginalStore {
    fn commit_temporary(
        &mut self,
        ticket: TempAssetCommitTicket,
    ) -> Result<AssetCommitOutcome, StudioError> {
        let mut committed = ticket.asset().clone();
        committed.commit_state = AssetCommitState::DurableOriginal;
        self.committed = Some(committed.clone());
        if self.acknowledgement_lost {
            Ok(AssetCommitOutcome::AcknowledgementLost)
        } else {
            Ok(AssetCommitOutcome::Committed(committed))
        }
    }

    fn probe_original(
        &mut self,
        _project_id: StudioProjectId,
        asset_id: StudioAssetId,
    ) -> Result<Option<StudioAsset>, StudioError> {
        Ok(self.committed.clone().filter(|asset| asset.id == asset_id))
    }

    fn delete_temporary(
        &mut self,
        _project_id: StudioProjectId,
        _asset_id: StudioAssetId,
        _expected_checksum: AssetChecksum,
    ) -> Result<(), StudioError> {
        Ok(())
    }
}

#[test]
fn temporary_asset_commit_reconciles_without_modifying_originals() {
    let mut temporary = asset(30, TrackKind::Screen);
    temporary.commit_state = AssetCommitState::Temporary;
    let ticket =
        TempAssetCommitTicket::new(project_id(31), operation_id(32), 7, temporary).expect("ticket");
    let mut store = FakeOriginalStore {
        acknowledgement_lost: true,
        ..FakeOriginalStore::default()
    };
    let committed = commit_verified_temporary(&mut store, ticket).expect("reconcile");
    assert_eq!(committed.commit_state, AssetCommitState::DurableOriginal);
}

#[test]
fn filesystem_original_and_project_stores_commit_atomically_and_preserve_originals() {
    let directory = tempfile::tempdir().expect("filesystem store directory");
    let originals_root = directory.path().join("original-store");
    let mut originals =
        FilesystemStudioOriginalStore::new(&originals_root).expect("original store");
    let bytes = b"isolated-screen-original";
    let mut temporary = asset(216, TrackKind::Screen);
    temporary.byte_len = bytes.len() as u64;
    temporary.checksum = AssetChecksum::from_content(bytes);
    temporary.commit_state = AssetCommitState::Temporary;
    originals
        .stage_temporary_bytes(project_id(217), &temporary, bytes)
        .expect("stage verified temporary");
    let committed = commit_verified_temporary(
        &mut originals,
        TempAssetCommitTicket::new(project_id(217), operation_id(218), 7, temporary.clone())
            .expect("commit ticket"),
    )
    .expect("atomic original commit");
    assert_eq!(committed.commit_state, AssetCommitState::DurableOriginal);
    assert_eq!(committed.checksum, temporary.checksum);
    assert_eq!(committed.byte_len, temporary.byte_len);
    assert_eq!(
        commit_verified_temporary(
            &mut originals,
            TempAssetCommitTicket::new(project_id(217), operation_id(219), 7, temporary,)
                .expect("replay ticket"),
        )
        .expect("idempotent original replay"),
        committed
    );

    // Simulate power loss after the same-filesystem rename but before the
    // canonical asset sidecar was persisted. Recovery must verify the orphaned
    // original and finish the sidecar without requiring the now-absent temp.
    let crash_bytes = b"rename-completed-before-sidecar";
    let mut crash_temporary = asset(222, TrackKind::Camera);
    crash_temporary.byte_len = crash_bytes.len() as u64;
    crash_temporary.checksum = AssetChecksum::from_content(crash_bytes);
    crash_temporary.commit_state = AssetCommitState::Temporary;
    originals
        .stage_temporary_bytes(project_id(217), &crash_temporary, crash_bytes)
        .expect("stage crash-window temporary");
    let project_stem = format!("{:02x}", 217).repeat(16);
    let asset_stem = format!("{:02x}", 222).repeat(16);
    let project_directory = originals_root.join(project_stem);
    fs::create_dir_all(project_directory.join("originals")).expect("create originals directory");
    let crash_original = project_directory
        .join("originals")
        .join(format!("{asset_stem}.media"));
    fs::rename(
        project_directory
            .join("temporary")
            .join(format!("{asset_stem}.media")),
        &crash_original,
    )
    .expect("simulate committed media rename");
    assert_eq!(
        fs::read(&crash_original).expect("read simulated original"),
        crash_bytes
    );
    let recovered = commit_verified_temporary(
        &mut originals,
        TempAssetCommitTicket::new(project_id(217), operation_id(223), 7, crash_temporary)
            .expect("crash recovery ticket"),
    )
    .expect("finish asset sidecar after rename crash");
    assert_eq!(recovered.commit_state, AssetCommitState::DurableOriginal);

    let projects_root = directory.path().join("project-store");
    let mut projects = FilesystemStudioProjectStore::new(&projects_root, 7).expect("project store");
    let current = manifest();
    projects
        .create_project(&current)
        .expect("create canonical project");
    let mut next_edits = current.edits.clone();
    next_edits.revision = current.revision + 1;
    let saved = commit_edit_save(
        &mut projects,
        EditSaveTicket::new(&current, operation_id(220), 7, next_edits).expect("edit ticket"),
    )
    .expect("atomic edit save");
    assert_eq!(saved.revision, current.revision + 1);
    assert_eq!(saved.assets, current.assets);
    assert_eq!(
        projects
            .probe_project(current.id)
            .expect("probe project")
            .expect("persisted project"),
        saved
    );

    let mut stale =
        FilesystemStudioProjectStore::new(&projects_root, 8).expect("stale project store");
    let mut stale_edits = saved.edits.clone();
    stale_edits.revision = saved.revision + 1;
    assert_eq!(
        commit_edit_save(
            &mut stale,
            EditSaveTicket::new(&saved, operation_id(221), 7, stale_edits).expect("stale ticket"),
        ),
        Err(StudioError::StaleJournal)
    );

    let mut claimed_edits = saved.edits.clone();
    claimed_edits.revision = saved.revision + 1;
    let claimed = commit_edit_save(
        &mut stale,
        EditSaveTicket::new(&saved, operation_id(224), 8, claimed_edits).expect("new-owner ticket"),
    )
    .expect("persist the newer durable fence");
    let mut old_owner_edits = claimed.edits.clone();
    old_owner_edits.revision = claimed.revision + 1;
    assert_eq!(
        commit_edit_save(
            &mut projects,
            EditSaveTicket::new(&claimed, operation_id(225), 7, old_owner_edits)
                .expect("old-owner ticket"),
        ),
        Err(StudioError::StaleJournal)
    );
}

#[test]
fn production_filesystem_path_records_previews_renders_and_recovers_terminal_receipt() {
    let workspace = tempfile::tempdir().expect("Studio workspace");
    let fixture = supported_cap_fixture();
    let assignment = LegacyIdAssignment {
        project_id: project_id(230),
        asset_ids: (231..=235).map(asset_id).collect(),
    };
    let mut cap = FilesystemLegacyCapProjectPort::open(&fixture).expect("Cap adapter");
    let LegacyImportOutcome::Imported(import) =
        import_legacy_cap(&mut cap, &assignment).expect("import plan")
    else {
        panic!("fixture must import");
    };

    let originals_root = workspace.path().join("originals");
    let mut originals =
        FilesystemStudioOriginalStore::new(&originals_root).expect("original store");
    for (index, (entry, durable)) in import
        .copy_plan
        .entries
        .iter()
        .zip(&import.manifest.assets)
        .enumerate()
    {
        let mut temporary = durable.clone();
        temporary.commit_state = AssetCommitState::Temporary;
        originals
            .stage_legacy_copy(&fixture, assignment.project_id, entry, &temporary)
            .expect("stream legacy original into isolated temporary");
        let committed = commit_verified_temporary(
            &mut originals,
            TempAssetCommitTicket::new(
                assignment.project_id,
                operation_id(236_u8.saturating_add(index as u8)),
                5,
                temporary,
            )
            .expect("original commit ticket"),
        )
        .expect("commit isolated original");
        assert_eq!(committed, *durable);
    }

    let mut project_store = FilesystemStudioProjectStore::new(workspace.path().join("projects"), 5)
        .expect("project store");
    project_store
        .create_project(&import.manifest)
        .expect("persist imported project");

    let mut coverage = import
        .manifest
        .assets
        .iter()
        .map(|asset| {
            Ok(SourceCoverage {
                track: asset.track,
                start: asset.start,
                end: RationalTime::new(
                    u64::try_from(asset.end()?.numerator())
                        .map_err(|_| StudioError::TimelineOverflow)?,
                    TimeBase::new(
                        u32::try_from(asset.end()?.denominator())
                            .map_err(|_| StudioError::TimelineOverflow)?,
                    )?,
                ),
            })
        })
        .collect::<Result<Vec<_>, StudioError>>()
        .expect("asset coverage");
    coverage.sort_by(|left, right| {
        left.track
            .cmp(&right.track)
            .then_with(|| left.start.ticks().cmp(&right.start.ticks()))
    });
    let timeline = TimelineSource {
        duration: seconds(9),
        coverage,
        vfr_video_pts: BTreeMap::new(),
    };
    let plan = StudioTimelineCompiler::compile(&timeline, &import.manifest.edits)
        .expect("canonical imported timeline");
    let sources =
        StudioSourceSet::from_project(&import.manifest, &timeline).expect("bound sources");
    let preview = StudioPreviewGraphSpec::compile(sources.clone(), plan.clone())
        .expect("executable preview graph");
    assert!(
        preview
            .seek(ExactDuration::new(1, 1).expect("seek"))
            .is_ok()
    );

    let capabilities = full_capabilities();
    let preflight = preflight_render(ExportProfile::DistributionMaster, &capabilities)
        .expect("render preflight");
    let graph = StudioRenderGraphSpec::compile(sources, plan, preflight).expect("render graph");
    let ticket = StudioRenderTicket::new(
        assignment.project_id,
        export_id(242),
        operation_id(243),
        5,
        StudioSourceName::new("studio-export.mp4").expect("output"),
        graph,
        Duration::from_secs(60),
    )
    .expect("render ticket");
    let journal_root = workspace.path().join("journals");
    let journal = DurableStudioJournal::create(
        FilesystemStudioJournalStore::new(&journal_root).expect("journal store"),
        prepared_render_snapshot(&ticket),
    )
    .expect("persist RenderPrepared");
    let dispatch = journal
        .into_render_authorization()
        .expect("journal authorization")
        .bind(ticket)
        .expect("bind render ticket");
    let exports_root = workspace.path().join("exports");
    let renderer =
        FilesystemStudioRenderer::new(&originals_root, &exports_root, capabilities.clone())
            .expect("filesystem renderer");
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        coordinator.start(dispatch).expect("execute export"),
        RenderSessionState::Running
    );
    assert_eq!(
        coordinator
            .poll(export_id(242), Duration::from_millis(1))
            .expect("persist terminal render"),
        RenderSessionState::Committed
    );
    let receipt = coordinator
        .receipt(operation_id(243))
        .cloned()
        .expect("structured render receipt");
    assert!(receipt.output_bytes > 0);
    coordinator
        .release_terminal(export_id(242))
        .expect("release durable terminal");

    let reopened = DurableStudioJournal::open(
        FilesystemStudioJournalStore::new(&journal_root).expect("reopen journal store"),
        assignment.project_id,
    )
    .expect("reopen committed render journal");
    assert_eq!(
        reopened.snapshot().boundary,
        JournalBoundary::RenderCommitted
    );
    assert_eq!(
        reopened
            .snapshot()
            .pending_render
            .as_ref()
            .and_then(|pending| pending.terminal_receipt.as_ref()),
        Some(&receipt)
    );
    let mut restarted_renderer =
        FilesystemStudioRenderer::new(&originals_root, &exports_root, capabilities)
            .expect("restart filesystem renderer");
    assert_eq!(
        restarted_renderer
            .probe(export_id(242))
            .expect("durable renderer postcondition"),
        RenderPostcondition::Committed {
            fence: receipt.fence,
            render_spec_digest: receipt.render_spec_digest,
            output_checksum: receipt.output_checksum,
            output_bytes: receipt.output_bytes,
        }
    );

    // A valid sidecar copied under another export identity must be corruption,
    // not proof that the queried export committed. Render-spec digests do not
    // include export IDs, so the filesystem adapter must bind the filename and
    // record explicitly.
    let committed_stem = format!("{:02x}", 242).repeat(16);
    let foreign_stem = format!("{:02x}", 244).repeat(16);
    fs::copy(
        exports_root.join(format!("{committed_stem}.render-receipt")),
        exports_root.join(format!("{foreign_stem}.render-receipt")),
    )
    .expect("copy receipt under foreign identity");
    assert_eq!(
        restarted_renderer.probe(export_id(244)),
        Err(StudioError::JournalCorrupt)
    );
}

#[derive(Debug, Default)]
struct FakeProjectStore {
    project: Option<StudioProjectManifest>,
    acknowledgement_lost: bool,
}

impl StudioProjectStorePort for FakeProjectStore {
    fn save_edits(&mut self, ticket: EditSaveTicket) -> Result<EditSaveOutcome, StudioError> {
        let project = ticket.next_project().clone();
        self.project = Some(project.clone());
        if self.acknowledgement_lost {
            Ok(EditSaveOutcome::AcknowledgementLost)
        } else {
            Ok(EditSaveOutcome::Committed(project))
        }
    }

    fn probe_project(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioProjectManifest>, StudioError> {
        Ok(self
            .project
            .clone()
            .filter(|project| project.id == project_id))
    }
}

#[test]
fn edit_save_lost_ack_reconciles_without_changing_original_assets() {
    let current = manifest();
    let originals = current.assets.clone();
    let mut edits = edit_spec();
    edits.revision = current.revision + 1;
    edits
        .operations
        .push(EditOperation::Split { at: seconds(7) });
    let ticket = EditSaveTicket::new(&current, operation_id(35), 9, edits).expect("save ticket");
    let mut store = FakeProjectStore {
        acknowledgement_lost: true,
        ..FakeProjectStore::default()
    };
    let committed = commit_edit_save(&mut store, ticket).expect("lost ack reconciliation");
    assert_eq!(committed.revision, current.revision + 1);
    assert_eq!(committed.assets, originals);
    assert_eq!(current.assets, originals);
}

#[test]
fn timeline_is_exact_and_preview_export_share_the_same_plan() {
    let plan = StudioTimelineCompiler::compile(&source(), &edit_spec()).expect("compile");
    assert_eq!(
        plan.output_duration,
        ExactDuration::new(6, 1).expect("duration")
    );
    assert!(plan.gaps.iter().any(|gap| {
        gap.track == TrackKind::Camera && gap.disposition == GapDisposition::HideCamera
    }));
    assert!(plan.gaps.iter().any(|gap| {
        gap.track == TrackKind::SystemAudio && gap.disposition == GapDisposition::InsertSilence
    }));

    let sources = StudioSourceSet::from_project(&manifest(), &source()).expect("sources");
    let preview = StudioPreviewGraphSpec::compile(sources.clone(), plan.clone()).expect("preview");
    let capabilities = full_capabilities();
    let preflight =
        preflight_render(ExportProfile::DistributionMaster, &capabilities).expect("preflight");
    let export = StudioRenderGraphSpec::compile(sources.clone(), plan.clone(), preflight)
        .expect("render graph");
    assert_eq!(preview.edit_plan_digest(), export.edit_plan_digest());
    assert_eq!(preview.plan, export.plan);
    assert_eq!(preview.sources, export.sources);

    let mut tampered = plan.clone();
    tampered.spans[0].style.cursor.hidden = !tampered.spans[0].style.cursor.hidden;
    assert_eq!(
        StudioPreviewGraphSpec::compile(sources, tampered),
        Err(StudioError::CorruptCompiledPlan)
    );

    let seek = preview
        .seek(ExactDuration::new(2, 1).expect("seek time"))
        .expect("seek");
    assert_eq!(seek.span_source_start, seconds(5));
    assert_eq!(seek.offset, ExactDuration::zero());

    let vfr = plan.mapped_vfr_points(TrackKind::Screen).expect("VFR map");
    assert!(vfr.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(!vfr.contains(&ExactDuration::new(3, 1).expect("deleted point")));

    let frames = simulate_cfr_timestamps(
        &plan,
        FrameRate {
            numerator: 2,
            denominator: 1,
        },
        20,
    )
    .expect("frames");
    assert_eq!(frames.len(), 12);
    assert_eq!(frames[0], ExactDuration::zero());
    assert_eq!(frames[11], ExactDuration::new(11, 2).expect("last frame"));

    let audio =
        simulate_audio_block_timestamps(&plan, 48_000, 480, 1_000).expect("audio timestamps");
    assert_eq!(audio.len(), 600);
    assert_eq!(audio[1], ExactDuration::new(1, 100).expect("10ms"));
}

#[test]
fn timeline_rejects_overlap_video_gaps_and_unbounded_simulation() {
    let mut overlapping = edit_spec();
    overlapping.operations.push(EditOperation::Speed {
        start: seconds(3),
        end: seconds(4),
        numerator: 1,
        denominator: 2,
    });
    assert_eq!(
        StudioTimelineCompiler::compile(&source(), &overlapping),
        Err(StudioError::OverlappingEdits)
    );

    let mut gap = source();
    gap.coverage[0].end = seconds(3);
    gap.vfr_video_pts.clear();
    assert_eq!(
        StudioTimelineCompiler::compile(&gap, &edit_spec()),
        Err(StudioError::UncoveredRequiredVideo)
    );

    let plan = StudioTimelineCompiler::compile(&source(), &edit_spec()).expect("plan");
    assert_eq!(
        simulate_cfr_timestamps(
            &plan,
            FrameRate {
                numerator: 60,
                denominator: 1,
            },
            2,
        ),
        Err(StudioError::SimulationLimitExceeded)
    );
    assert_eq!(
        ExactDuration::new(u128::MAX, 1)
            .expect("maximum duration")
            .checked_add(ExactDuration::new(1, 1).expect("one second")),
        Err(StudioError::TimelineOverflow)
    );
}

#[test]
fn source_set_binds_assets_timeline_and_saved_edits_exactly() {
    let project = manifest();
    let timeline = source();
    let sources = StudioSourceSet::from_project(&project, &timeline).expect("bound sources");

    let mut missing_claimed_track = project.clone();
    missing_claimed_track
        .assets
        .retain(|asset| asset.track != TrackKind::SystemAudio);
    assert_eq!(
        StudioSourceSet::from_project(&missing_claimed_track, &timeline),
        Err(StudioError::SourceSetTimelineMismatch)
    );

    let mut unrelated_edits = project.edits.clone();
    let cursor = unrelated_edits
        .operations
        .iter_mut()
        .find_map(|operation| match operation {
            EditOperation::CursorTransform { scale_milli, .. } => Some(scale_milli),
            _ => None,
        })
        .expect("cursor edit");
    *cursor = 1_750;
    let unrelated_plan =
        StudioTimelineCompiler::compile(&timeline, &unrelated_edits).expect("unrelated plan");
    let preflight = preflight_render(ExportProfile::DistributionMaster, &full_capabilities())
        .expect("preflight");
    assert_eq!(
        StudioRenderGraphSpec::compile(sources, unrelated_plan, preflight),
        Err(StudioError::SourceSetTimelineMismatch)
    );

    let segmented_timeline = TimelineSource {
        duration: seconds(10),
        coverage: vec![
            SourceCoverage {
                track: TrackKind::Screen,
                start: seconds(0),
                end: seconds(5),
            },
            SourceCoverage {
                track: TrackKind::Screen,
                start: seconds(5),
                end: seconds(10),
            },
        ],
        vfr_video_pts: BTreeMap::new(),
    };
    let segmented_project = StudioProjectManifest {
        version: STUDIO_PROJECT_VERSION,
        id: project_id(150),
        revision: 1,
        state: StudioState::Editing,
        assets: vec![
            asset_with_range(151, TrackKind::Screen, 0, 5),
            asset_with_range(152, TrackKind::Screen, 5, 5),
        ],
        edits: EditSpec::default(),
    };
    let segmented_sources = StudioSourceSet::from_project(&segmented_project, &segmented_timeline)
        .expect("contiguous segments");
    let segmented_plan = StudioTimelineCompiler::compile(&segmented_timeline, &EditSpec::default())
        .expect("segmented plan");
    StudioRenderGraphSpec::compile(segmented_sources, segmented_plan, preflight)
        .expect("segmented render graph");

    let mut gap = segmented_project.clone();
    gap.assets[1].start = seconds(6);
    assert_eq!(gap.validate(), Err(StudioError::InvalidSourceSet));
    let mut overlap = segmented_project;
    overlap.assets[1].start = seconds(4);
    assert_eq!(overlap.validate(), Err(StudioError::InvalidSourceSet));
}

#[test]
fn edit_save_rejects_operations_outside_the_outer_trim() {
    let mut invalid = edit_spec();
    let layout = invalid
        .operations
        .iter_mut()
        .find_map(|operation| match operation {
            EditOperation::Layout { start, end, .. } => Some((start, end)),
            _ => None,
        })
        .expect("layout edit");
    *layout.0 = seconds(0);
    *layout.1 = seconds(1);
    assert_eq!(
        StudioDocumentCodec::encode_edit(&invalid),
        Err(StudioError::EditOutsideTimeline)
    );

    let mut boundary_split = edit_spec();
    let split = boundary_split
        .operations
        .iter_mut()
        .find_map(|operation| match operation {
            EditOperation::Split { at } => Some(at),
            _ => None,
        })
        .expect("split edit");
    *split = seconds(1);
    assert_eq!(
        StudioDocumentCodec::encode_edit(&boundary_split),
        Err(StudioError::EditOutsideTimeline)
    );

    invalid.revision = manifest().revision + 1;
    assert!(matches!(
        EditSaveTicket::new(&manifest(), operation_id(153), 4, invalid),
        Err(StudioError::EditOutsideTimeline)
    ));
}

#[test]
fn edit_save_rejects_a_fully_deleted_output_before_persistence() {
    let current = manifest();
    let edits = EditSpec {
        version: STUDIO_EDIT_VERSION,
        revision: current.revision + 1,
        operations: vec![
            EditOperation::Trim {
                start: seconds(1),
                end: seconds(9),
            },
            EditOperation::DeleteRange {
                start: seconds(1),
                end: seconds(9),
            },
        ],
    };
    assert!(matches!(
        EditSaveTicket::new(&current, operation_id(154), 4, edits.clone()),
        Err(StudioError::EmptyOutput)
    ));
    assert_eq!(
        StudioTimelineCompiler::compile(&source(), &edits),
        Err(StudioError::EmptyOutput)
    );
}

#[test]
fn media_queue_bytes_and_time_have_hard_policy_ceilings() {
    assert_eq!(
        BoundedMediaQueue {
            max_buffers: 1,
            max_bytes: MAX_STUDIO_QUEUE_BYTES + 1,
            max_time_ns: 1,
        }
        .validate(),
        Err(StudioError::UnboundedMediaQueue)
    );
    assert_eq!(
        BoundedMediaQueue {
            max_buffers: 1,
            max_bytes: 1,
            max_time_ns: MAX_STUDIO_QUEUE_TIME_NS + 1,
        }
        .validate(),
        Err(StudioError::UnboundedMediaQueue)
    );
    BoundedMediaQueue {
        max_buffers: MAX_STUDIO_QUEUE_BUFFERS,
        max_bytes: MAX_STUDIO_QUEUE_BYTES,
        max_time_ns: MAX_STUDIO_QUEUE_TIME_NS,
    }
    .validate()
    .expect("exact policy ceilings");
}

#[test]
fn day_long_project_compiles_and_seeks_without_linear_allocation() {
    let duration = seconds(24 * 60 * 60);
    let long_source = TimelineSource {
        duration,
        coverage: vec![SourceCoverage {
            track: TrackKind::Screen,
            start: seconds(0),
            end: duration,
        }],
        vfr_video_pts: BTreeMap::new(),
    };
    let plan =
        StudioTimelineCompiler::compile(&long_source, &EditSpec::default()).expect("day-long plan");
    assert_eq!(plan.spans.len(), 1);
    assert_eq!(
        plan.output_duration,
        ExactDuration::new(86_400, 1).expect("one day")
    );
    let seek = plan
        .seek(ExactDuration::new(86_399, 1).expect("near end"))
        .expect("near-end seek");
    assert_eq!(seek.offset, ExactDuration::new(86_399, 1).expect("offset"));
    assert_eq!(
        simulate_cfr_timestamps(
            &plan,
            FrameRate {
                numerator: 60,
                denominator: 1,
            },
            10_000,
        ),
        Err(StudioError::SimulationLimitExceeded)
    );
}

fn full_capabilities() -> RenderCapabilities {
    RenderCapabilities {
        contract_version: STUDIO_RENDER_PROTOCOL_VERSION,
        containers: BTreeSet::from([
            MediaContainer::Mp4,
            MediaContainer::WebM,
            MediaContainer::Matroska,
        ]),
        hardware_video: BTreeSet::from([StudioVideoCodec::H264Avc, StudioVideoCodec::H265Hevc]),
        software_video: BTreeSet::from([
            StudioVideoCodec::H264Avc,
            StudioVideoCodec::H265Hevc,
            StudioVideoCodec::Vp8,
            StudioVideoCodec::Ffv1,
        ]),
        audio: BTreeSet::from([
            StudioAudioCodec::AacLowComplexity,
            StudioAudioCodec::Opus,
            StudioAudioCodec::Flac,
        ]),
        licenses: BTreeSet::from([
            CodecLicense::H264Encode,
            CodecLicense::H265Encode,
            CodecLicense::AacEncode,
        ]),
        maximum_resolution: Resolution {
            width: 3_840,
            height: 2_160,
        },
        maximum_frame_rate: FrameRate {
            numerator: 60,
            denominator: 1,
        },
        bounded_renderer_queue: true,
        cancellation: true,
        postcondition_probe: true,
        exact_partial_cleanup: true,
    }
}

#[test]
fn render_preflight_is_exact_about_licenses_and_fallback() {
    let capabilities = full_capabilities();
    let distribution =
        preflight_render(ExportProfile::DistributionMaster, &capabilities).expect("distribution");
    assert!(distribution.profile.hosted_distribution_compatible);
    assert_eq!(distribution.selected_backend, EncoderBackend::Hardware);
    assert_eq!(
        hardware_failure_disposition(distribution),
        HardwareFailureDisposition::RestartFromCleanPartialWithSoftware
    );

    let mut unlicensed = capabilities;
    unlicensed.licenses.remove(&CodecLicense::H265Encode);
    assert_eq!(
        preflight_render(ExportProfile::NativeHighQualityHevc, &unlicensed),
        Err(StudioError::MissingCodecLicense(CodecLicense::H265Encode))
    );

    let plan = StudioTimelineCompiler::compile(&source(), &edit_spec()).expect("plan");
    let mut source_project = manifest();
    source_project.id = project_id(60);
    let sources = StudioSourceSet::from_project(&source_project, &source()).expect("sources");
    let mut graph = StudioRenderGraphSpec::compile(sources, plan, distribution).expect("graph");
    graph.nodes.swap(0, 1);
    assert!(matches!(
        StudioRenderTicket::new(
            project_id(60),
            export_id(61),
            operation_id(62),
            1,
            StudioSourceName::new("invalid-graph.mp4").expect("name"),
            graph,
            Duration::from_secs(60),
        ),
        Err(StudioError::InvalidRenderGraph)
    ));
}

#[derive(Debug)]
struct MemoryPayload {
    declared: u64,
    chunks: VecDeque<Vec<u8>>,
    cancellations: Rc<Cell<usize>>,
}

impl StudioOneShotPayload for MemoryPayload {
    fn declared_len(&self) -> u64 {
        self.declared
    }

    fn pull(&mut self, _maximum_bytes: usize) -> Result<Option<Vec<u8>>, StudioError> {
        Ok(self.chunks.pop_front())
    }

    fn cancel(&mut self) {
        self.cancellations
            .set(self.cancellations.get().saturating_add(1));
    }
}

#[test]
fn one_shot_payload_is_bounded_and_integrity_checked() {
    let bytes = b"bounded-control-payload".to_vec();
    let cancellations = Rc::new(Cell::new(0));
    let mut payload = MemoryPayload {
        declared: bytes.len() as u64,
        chunks: bytes.chunks(3).map(<[u8]>::to_vec).collect(),
        cancellations: Rc::clone(&cancellations),
    };
    assert_eq!(
        consume_bounded_control_payload(&mut payload, AssetChecksum::from_content(&bytes), 1_024,)
            .expect("payload"),
        bytes
    );
    assert_eq!(cancellations.get(), 0);

    let mut extra = MemoryPayload {
        declared: 1,
        chunks: VecDeque::from([vec![1, 2]]),
        cancellations: Rc::clone(&cancellations),
    };
    assert_eq!(
        consume_bounded_control_payload(&mut extra, checksum(1), 8),
        Err(StudioError::PayloadLengthMismatch)
    );
    assert_eq!(cancellations.get(), 1);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeStartMode {
    Accepted,
    LoseAckRunning,
    LoseAckPartial,
    LoseAckAbsent,
}

#[derive(Debug, Clone)]
struct ActiveRender {
    fence: u64,
    render_spec_digest: Sha256Digest,
}

#[derive(Debug)]
struct FakeRenderState {
    start_mode: FakeStartMode,
    fail_next_probe: bool,
    cleanup_confirms_absence: bool,
    mismatch_committed_postcondition: bool,
    active: BTreeMap<StudioExportId, ActiveRender>,
    postconditions: BTreeMap<StudioExportId, RenderPostcondition>,
    events: BTreeMap<StudioExportId, VecDeque<RenderEvent>>,
    cleanups: Vec<(StudioExportId, u64, Sha256Digest, StudioSourceName)>,
    cancellations: Vec<(StudioExportId, u64)>,
}

impl Default for FakeRenderState {
    fn default() -> Self {
        Self {
            start_mode: FakeStartMode::Accepted,
            fail_next_probe: false,
            cleanup_confirms_absence: true,
            mismatch_committed_postcondition: false,
            active: BTreeMap::new(),
            postconditions: BTreeMap::new(),
            events: BTreeMap::new(),
            cleanups: Vec::new(),
            cancellations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct FakeRenderer {
    capabilities: RenderCapabilities,
    state: Rc<RefCell<FakeRenderState>>,
}

impl FakeRenderer {
    fn new() -> (Self, Rc<RefCell<FakeRenderState>>) {
        let state = Rc::new(RefCell::new(FakeRenderState::default()));
        (
            Self {
                capabilities: full_capabilities(),
                state: Rc::clone(&state),
            },
            state,
        )
    }
}

impl StudioRendererPort for FakeRenderer {
    fn capabilities(&mut self) -> Result<RenderCapabilities, StudioError> {
        Ok(self.capabilities.clone())
    }

    fn start(&mut self, ticket: StudioRenderTicket) -> Result<RenderStartOutcome, StudioError> {
        let active = ActiveRender {
            fence: ticket.expected_fence(),
            render_spec_digest: ticket.render_spec_digest(),
        };
        let mut state = self.state.borrow_mut();
        state.active.insert(ticket.export_id(), active.clone());
        match state.start_mode {
            FakeStartMode::Accepted => {
                state.postconditions.insert(
                    ticket.export_id(),
                    RenderPostcondition::Running {
                        fence: active.fence,
                        render_spec_digest: active.render_spec_digest,
                    },
                );
                Ok(RenderStartOutcome::Accepted)
            }
            FakeStartMode::LoseAckRunning => {
                state.postconditions.insert(
                    ticket.export_id(),
                    RenderPostcondition::Running {
                        fence: active.fence,
                        render_spec_digest: active.render_spec_digest,
                    },
                );
                Ok(RenderStartOutcome::AcknowledgementLost)
            }
            FakeStartMode::LoseAckPartial => {
                state.postconditions.insert(
                    ticket.export_id(),
                    RenderPostcondition::Partial {
                        fence: active.fence,
                        render_spec_digest: active.render_spec_digest,
                    },
                );
                Ok(RenderStartOutcome::AcknowledgementLost)
            }
            FakeStartMode::LoseAckAbsent => {
                state
                    .postconditions
                    .insert(ticket.export_id(), RenderPostcondition::Absent);
                Ok(RenderStartOutcome::AcknowledgementLost)
            }
        }
    }

    fn poll(
        &mut self,
        export_id: StudioExportId,
        maximum_events: usize,
        _wait: Duration,
    ) -> Result<Vec<RenderEvent>, StudioError> {
        let mut state = self.state.borrow_mut();
        let mut output = Vec::new();
        for _ in 0..maximum_events {
            let event = state
                .events
                .get_mut(&export_id)
                .and_then(VecDeque::pop_front);
            let Some(event) = event else {
                break;
            };
            let active = state
                .active
                .get(&export_id)
                .cloned()
                .ok_or(StudioError::UnknownExport)?;
            match event.kind {
                RenderEventKind::Committed {
                    output_checksum,
                    output_bytes,
                } => {
                    let fence = if state.mismatch_committed_postcondition {
                        active.fence.saturating_add(1)
                    } else {
                        active.fence
                    };
                    state.postconditions.insert(
                        export_id,
                        RenderPostcondition::Committed {
                            fence,
                            render_spec_digest: active.render_spec_digest,
                            output_checksum,
                            output_bytes,
                        },
                    );
                }
                RenderEventKind::Cancelled | RenderEventKind::Failed { .. } => {
                    state.postconditions.insert(
                        export_id,
                        RenderPostcondition::Partial {
                            fence: active.fence,
                            render_spec_digest: active.render_spec_digest,
                        },
                    );
                }
                RenderEventKind::Progress { .. } => {}
            }
            output.push(event);
        }
        Ok(output)
    }

    fn probe(&mut self, export_id: StudioExportId) -> Result<RenderPostcondition, StudioError> {
        let mut state = self.state.borrow_mut();
        if std::mem::take(&mut state.fail_next_probe) {
            return Err(StudioError::IncompatibleRenderer);
        }
        Ok(state
            .postconditions
            .get(&export_id)
            .cloned()
            .unwrap_or(RenderPostcondition::Absent))
    }

    fn cancel(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        _deadline: Duration,
    ) -> Result<(), StudioError> {
        let mut state = self.state.borrow_mut();
        state.cancellations.push((export_id, expected_fence));
        if let Some(active) = state.active.get(&export_id).cloned() {
            state.postconditions.insert(
                export_id,
                RenderPostcondition::Partial {
                    fence: active.fence,
                    render_spec_digest: active.render_spec_digest,
                },
            );
        }
        Ok(())
    }

    fn cleanup_partial(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        expected_render_spec_digest: Sha256Digest,
        output_name: &StudioSourceName,
    ) -> Result<(), StudioError> {
        let mut state = self.state.borrow_mut();
        state.cleanups.push((
            export_id,
            expected_fence,
            expected_render_spec_digest,
            output_name.clone(),
        ));
        if state.cleanup_confirms_absence {
            state
                .postconditions
                .insert(export_id, RenderPostcondition::Absent);
        }
        Ok(())
    }
}

fn render_ticket(
    capabilities: &RenderCapabilities,
    export_marker: u8,
    operation_marker: u8,
    backend: EncoderBackend,
) -> AuthorizedRenderDispatch {
    render_ticket_to(
        capabilities,
        export_marker,
        operation_marker,
        backend,
        export_marker,
    )
}

fn render_ticket_to(
    capabilities: &RenderCapabilities,
    export_marker: u8,
    operation_marker: u8,
    backend: EncoderBackend,
    output_marker: u8,
) -> AuthorizedRenderDispatch {
    let mut source_project = manifest();
    source_project.id = project_id(80);
    render_ticket_for_project(
        capabilities,
        export_marker,
        operation_marker,
        backend,
        output_marker,
        source_project,
        source(),
    )
}

fn unbound_render_ticket_to(
    capabilities: &RenderCapabilities,
    export_marker: u8,
    operation_marker: u8,
    backend: EncoderBackend,
    output_marker: u8,
) -> StudioRenderTicket {
    let mut source_project = manifest();
    source_project.id = project_id(80);
    unbound_render_ticket_for_project(
        capabilities,
        export_marker,
        operation_marker,
        backend,
        output_marker,
        source_project,
        source(),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_ticket_for_project(
    capabilities: &RenderCapabilities,
    export_marker: u8,
    operation_marker: u8,
    backend: EncoderBackend,
    output_marker: u8,
    source_project: StudioProjectManifest,
    timeline: TimelineSource,
) -> AuthorizedRenderDispatch {
    authorize_ticket(unbound_render_ticket_for_project(
        capabilities,
        export_marker,
        operation_marker,
        backend,
        output_marker,
        source_project,
        timeline,
    ))
}

#[allow(clippy::too_many_arguments)]
fn unbound_render_ticket_for_project(
    capabilities: &RenderCapabilities,
    export_marker: u8,
    operation_marker: u8,
    backend: EncoderBackend,
    output_marker: u8,
    source_project: StudioProjectManifest,
    timeline: TimelineSource,
) -> StudioRenderTicket {
    let plan = StudioTimelineCompiler::compile(&timeline, &source_project.edits).expect("plan");
    let mut preflight =
        preflight_render(ExportProfile::DistributionMaster, capabilities).expect("preflight");
    preflight.selected_backend = backend;
    let sources = StudioSourceSet::from_project(&source_project, &timeline).expect("sources");
    let graph = StudioRenderGraphSpec::compile(sources, plan, preflight).expect("graph");
    StudioRenderTicket::new(
        project_id(80),
        export_id(export_marker),
        operation_id(operation_marker),
        5,
        StudioSourceName::new(format!("export-{output_marker}.mp4")).expect("output"),
        graph,
        Duration::from_secs(3_600),
    )
    .expect("ticket")
}

fn authorize_ticket(ticket: StudioRenderTicket) -> AuthorizedRenderDispatch {
    let snapshot = prepared_render_snapshot(&ticket);
    DurableStudioJournal::create(FakeJournal::default(), snapshot)
        .expect("durable render reservation")
        .into_render_authorization()
        .expect("render authorization")
        .bind(ticket)
        .expect("authorization matches ticket")
}

fn prepared_render_snapshot(ticket: &StudioRenderTicket) -> StudioJournalSnapshot {
    let pending = PendingRender::new(
        ticket.operation_id(),
        ticket.export_id(),
        ticket.expected_fence(),
        ticket.graph().sources.digest(),
        ticket.graph().edit_plan_digest(),
        ticket.graph().preflight.profile.profile,
        ticket.output_name().clone(),
    )
    .expect("pending render");
    StudioJournalSnapshot {
        version: STUDIO_JOURNAL_VERSION,
        project_id: ticket.project_id(),
        revision: 1,
        fence: ticket.expected_fence(),
        owner: worker_id(198),
        boundary: JournalBoundary::RenderPrepared,
        last_operation_id: Some(ticket.operation_id()),
        pending_asset: None,
        pending_edit: None,
        pending_render: Some(pending),
        receipts: BTreeMap::from([(
            ticket.operation_id(),
            StudioOperationReceipt {
                operation_id: ticket.operation_id(),
                kind: ReceiptKind::RenderPrepared,
                command_digest: strong_sha256(b"render prepared command"),
                outcome_digest: strong_sha256(b"render prepared outcome"),
            },
        )]),
    }
}

fn committed_recovery_reservation(
    ticket: &StudioRenderTicket,
    output_checksum: AssetChecksum,
    output_bytes: u64,
) -> RenderJournalAuthorization {
    let receipt = RenderReceipt {
        project_id: ticket.project_id(),
        export_id: ticket.export_id(),
        operation_id: ticket.operation_id(),
        fence: ticket.expected_fence(),
        source_set_digest: ticket.graph().sources.digest(),
        plan_digest: ticket.graph().edit_plan_digest(),
        render_spec_digest: ticket.render_spec_digest(),
        profile: ticket.graph().preflight.profile,
        output_name: ticket.output_name().clone(),
        output_checksum,
        output_bytes,
    };
    let mut pending = PendingRender::new(
        ticket.operation_id(),
        ticket.export_id(),
        ticket.expected_fence(),
        ticket.graph().sources.digest(),
        ticket.graph().edit_plan_digest(),
        ticket.graph().preflight.profile.profile,
        ticket.output_name().clone(),
    )
    .expect("pending render");
    pending.terminal_receipt = Some(receipt);
    let snapshot = StudioJournalSnapshot {
        version: STUDIO_JOURNAL_VERSION,
        project_id: ticket.project_id(),
        revision: 4,
        fence: ticket.expected_fence(),
        owner: worker_id(199),
        boundary: JournalBoundary::RenderCommitted,
        last_operation_id: Some(ticket.operation_id()),
        pending_asset: None,
        pending_edit: None,
        pending_render: Some(pending),
        receipts: BTreeMap::from([(
            ticket.operation_id(),
            StudioOperationReceipt {
                operation_id: ticket.operation_id(),
                kind: ReceiptKind::RenderCommitted,
                command_digest: strong_sha256(b"recovered render commit"),
                outcome_digest: strong_sha256(b"recovered render terminal receipt"),
            },
        )]),
    };
    DurableStudioJournal::create(FakeJournal::default(), snapshot)
        .expect("persisted committed render journal")
        .into_render_authorization()
        .expect("committed recovery authorization")
}

fn push_event(
    state: &Rc<RefCell<FakeRenderState>>,
    export_marker: u8,
    sequence: u64,
    fence: u64,
    kind: RenderEventKind,
) {
    let render_spec_digest = state
        .borrow()
        .active
        .get(&export_id(export_marker))
        .expect("active render")
        .render_spec_digest;
    state
        .borrow_mut()
        .events
        .entry(export_id(export_marker))
        .or_default()
        .push_back(RenderEvent {
            project_id: project_id(80),
            export_id: export_id(export_marker),
            fence,
            render_spec_digest,
            sequence,
            kind,
        });
}

#[test]
fn renderer_reconciles_lost_start_ack_and_verifies_commit_postcondition() {
    let (renderer, state) = FakeRenderer::new();
    state.borrow_mut().start_mode = FakeStartMode::LoseAckRunning;
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        coordinator
            .start(render_ticket(
                &capabilities,
                81,
                82,
                EncoderBackend::Hardware,
            ))
            .expect("start"),
        RenderSessionState::Running
    );
    push_event(
        &state,
        81,
        1,
        5,
        RenderEventKind::Progress {
            phase: RenderPhase::Finalizing,
            basis_points: 10_000,
        },
    );
    push_event(
        &state,
        81,
        2,
        5,
        RenderEventKind::Committed {
            output_checksum: checksum(83),
            output_bytes: 9_999,
        },
    );
    assert_eq!(
        coordinator
            .poll(export_id(81), Duration::from_millis(1))
            .expect("poll"),
        RenderSessionState::Committed
    );
    let receipt = coordinator
        .receipt(operation_id(82))
        .expect("durable receipt");
    assert_eq!(receipt.output_checksum, checksum(83));
    assert_eq!(receipt.output_bytes, 9_999);
}

#[test]
fn ambiguous_partial_start_is_cleaned_before_error() {
    let (renderer, state) = FakeRenderer::new();
    state.borrow_mut().start_mode = FakeStartMode::LoseAckPartial;
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        coordinator.start(render_ticket(
            &capabilities,
            84,
            85,
            EncoderBackend::Hardware,
        )),
        Err(StudioError::AmbiguousRenderStart)
    );
    assert_eq!(
        state.borrow().postconditions.get(&export_id(84)).cloned(),
        Some(RenderPostcondition::Absent)
    );
    assert_eq!(state.borrow().cleanups.len(), 1);
}

#[test]
fn lost_ack_absence_stays_quarantined_through_delayed_publication() {
    for (offset, delayed_running) in [(0_u8, true), (4_u8, false)] {
        let (renderer, state) = FakeRenderer::new();
        state.borrow_mut().start_mode = FakeStartMode::LoseAckAbsent;
        let capabilities = renderer.capabilities.clone();
        let mut coordinator =
            StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
        let export_marker = 200_u8.saturating_add(offset);
        let operation_marker = export_marker.saturating_add(1);
        let output_marker = export_marker.saturating_add(2);
        assert_eq!(
            coordinator.start(render_ticket_to(
                &capabilities,
                export_marker,
                operation_marker,
                EncoderBackend::Hardware,
                output_marker,
            )),
            Err(StudioError::AmbiguousRenderStart)
        );
        assert_eq!(
            coordinator.release_terminal(export_id(export_marker)),
            Err(StudioError::PartialCleanupUnconfirmed)
        );
        assert_eq!(
            coordinator.start(render_ticket_to(
                &capabilities,
                export_marker.saturating_add(3),
                operation_marker.saturating_add(3),
                EncoderBackend::Hardware,
                output_marker,
            )),
            Err(StudioError::OutputTargetBusy)
        );

        let active = state
            .borrow()
            .active
            .get(&export_id(export_marker))
            .cloned()
            .expect("lost-ack execution identity retained");
        let delayed = if delayed_running {
            RenderPostcondition::Running {
                fence: active.fence,
                render_spec_digest: active.render_spec_digest,
            }
        } else {
            RenderPostcondition::Partial {
                fence: active.fence,
                render_spec_digest: active.render_spec_digest,
            }
        };
        state
            .borrow_mut()
            .postconditions
            .insert(export_id(export_marker), delayed);

        coordinator
            .cancel_and_cleanup(export_id(export_marker), Duration::from_secs(1))
            .expect("exact fenced cancel and cleanup");
        assert_eq!(
            state.borrow().cancellations.last(),
            Some(&(export_id(export_marker), 5))
        );
        coordinator
            .release_terminal(export_id(export_marker))
            .expect("release after terminal proof");
    }
}

#[test]
fn acknowledgement_loss_keeps_output_reserved_through_probe_and_cleanup_uncertainty() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    {
        let mut control = state.borrow_mut();
        control.start_mode = FakeStartMode::LoseAckPartial;
        control.cleanup_confirms_absence = false;
    }
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            140,
            141,
            EncoderBackend::Hardware,
            142,
        )),
        Err(StudioError::PartialCleanupUnconfirmed)
    );
    assert_eq!(
        coordinator.release_terminal(export_id(140)),
        Err(StudioError::PartialCleanupUnconfirmed)
    );
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            143,
            144,
            EncoderBackend::Hardware,
            142,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    {
        let mut control = state.borrow_mut();
        control.cleanup_confirms_absence = true;
        control.start_mode = FakeStartMode::Accepted;
    }
    coordinator
        .cancel_and_cleanup(export_id(140), Duration::from_secs(1))
        .expect("retry exact cleanup");
    coordinator
        .release_terminal(export_id(140))
        .expect("release confirmed cleanup");
    coordinator
        .start(render_ticket_to(
            &capabilities,
            143,
            144,
            EncoderBackend::Hardware,
            142,
        ))
        .expect("reuse only after confirmed cleanup");

    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    {
        let mut control = state.borrow_mut();
        control.start_mode = FakeStartMode::LoseAckRunning;
        control.fail_next_probe = true;
    }
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            145,
            146,
            EncoderBackend::Hardware,
            147,
        )),
        Err(StudioError::IncompatibleRenderer)
    );
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            148,
            149,
            EncoderBackend::Hardware,
            147,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    coordinator
        .cancel_and_cleanup(export_id(145), Duration::from_secs(1))
        .expect("clean after probe uncertainty");
}

#[test]
fn committed_postcondition_mismatch_cannot_release_or_reuse_the_output() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    state.borrow_mut().mismatch_committed_postcondition = true;
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket_to(
            &capabilities,
            150,
            151,
            EncoderBackend::Hardware,
            152,
        ))
        .expect("start");
    push_event(
        &state,
        150,
        1,
        5,
        RenderEventKind::Progress {
            phase: RenderPhase::Finalizing,
            basis_points: 10_000,
        },
    );
    push_event(
        &state,
        150,
        2,
        5,
        RenderEventKind::Committed {
            output_checksum: checksum(153),
            output_bytes: 12_345,
        },
    );
    assert_eq!(
        coordinator.poll(export_id(150), Duration::from_millis(1)),
        Err(StudioError::RenderPostconditionMismatch)
    );
    assert_eq!(
        coordinator.release_terminal(export_id(150)),
        Err(StudioError::PartialCleanupUnconfirmed)
    );
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            154,
            155,
            EncoderBackend::Hardware,
            152,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    coordinator
        .cancel_and_cleanup(export_id(150), Duration::from_secs(1))
        .expect("cleanup mismatched commit quarantine");
}

#[test]
fn coordinator_restart_reconstructs_output_reservation_from_the_journal() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let directory = tempfile::tempdir().expect("journal directory");
    let ticket = unbound_render_ticket_to(&capabilities, 180, 181, EncoderBackend::Hardware, 182);
    let project = ticket.project_id();
    let journal = DurableStudioJournal::create(
        FilesystemStudioJournalStore::new(directory.path()).expect("filesystem journal store"),
        prepared_render_snapshot(&ticket),
    )
    .expect("persist render reservation");
    let dispatch = journal
        .into_render_authorization()
        .expect("durable dispatch authorization")
        .bind(ticket)
        .expect("bind persisted ticket");
    let mut original = StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    original.start(dispatch).expect("original start");
    let renderer = original.into_renderer();

    let reopened = DurableStudioJournal::open(
        FilesystemStudioJournalStore::new(directory.path()).expect("reopened filesystem store"),
        project,
    )
    .expect("reopen real persisted journal");
    assert_eq!(reopened.snapshot().boundary, JournalBoundary::RenderRunning);
    let reservation = reopened
        .into_render_authorization()
        .expect("recovered durable authorization");

    let mut restarted = StudioRenderCoordinator::new(renderer, 8, vec![reservation])
        .expect("restart with journal reservations");
    assert_eq!(
        restarted.start(render_ticket_to(
            &capabilities,
            180,
            181,
            EncoderBackend::Hardware,
            182,
        )),
        Err(StudioError::RecoveredRenderRequiresReconciliation)
    );
    assert_eq!(
        restarted.start(render_ticket_to(
            &capabilities,
            184,
            185,
            EncoderBackend::Hardware,
            182,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    assert_eq!(
        restarted
            .reconcile_recovered_cleanup(export_id(180), Duration::from_secs(1))
            .expect("reconcile recovered partial"),
        RenderSessionState::Cancelled
    );
    assert_eq!(state.borrow().cleanups.len(), 1);
    assert_eq!(
        restarted.start(render_ticket_to(
            &capabilities,
            184,
            185,
            EncoderBackend::Hardware,
            182,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    restarted
        .release_terminal(export_id(180))
        .expect("explicit recovered release");
    restarted
        .start(render_ticket_to(
            &capabilities,
            184,
            185,
            EncoderBackend::Hardware,
            182,
        ))
        .expect("reuse after recovered release");
}

#[test]
fn recovered_lost_ack_absence_requires_fenced_cancel_before_release() {
    let (renderer, state) = FakeRenderer::new();
    state.borrow_mut().start_mode = FakeStartMode::LoseAckAbsent;
    let capabilities = renderer.capabilities.clone();
    let directory = tempfile::tempdir().expect("journal directory");
    let ticket = unbound_render_ticket_to(&capabilities, 194, 195, EncoderBackend::Hardware, 196);
    let project = ticket.project_id();
    let journal = DurableStudioJournal::create(
        FilesystemStudioJournalStore::new(directory.path()).expect("filesystem journal store"),
        prepared_render_snapshot(&ticket),
    )
    .expect("persist render reservation");
    let dispatch = journal
        .into_render_authorization()
        .expect("durable dispatch authorization")
        .bind(ticket)
        .expect("bind persisted ticket");
    let mut original = StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        original.start(dispatch),
        Err(StudioError::AmbiguousRenderStart)
    );
    let renderer = original.into_renderer();

    let reopened = DurableStudioJournal::open(
        FilesystemStudioJournalStore::new(directory.path()).expect("reopened filesystem store"),
        project,
    )
    .expect("reopen failed-recoverably journal");
    assert_eq!(
        reopened.snapshot().boundary,
        JournalBoundary::FailedRecoverably
    );
    let authorization = reopened
        .into_render_authorization()
        .expect("recovered authorization");
    let mut restarted = StudioRenderCoordinator::new(renderer, 8, vec![authorization])
        .expect("restart with durable reservation");
    assert_eq!(
        restarted
            .reconcile_recovered_cleanup(export_id(194), Duration::from_secs(1))
            .expect("fenced recovered cleanup"),
        RenderSessionState::Cancelled
    );
    let control = state.borrow();
    assert_eq!(control.cancellations, vec![(export_id(194), 5)]);
    assert_eq!(control.cleanups.len(), 1);
    assert_eq!(
        control.postconditions.get(&export_id(194)),
        Some(&RenderPostcondition::Absent)
    );
}

#[test]
fn restart_journalizes_an_exact_renderer_commit_before_adoption() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let directory = tempfile::tempdir().expect("journal directory");
    let ticket = unbound_render_ticket_to(&capabilities, 197, 198, EncoderBackend::Hardware, 199);
    let project = ticket.project_id();
    let journal = DurableStudioJournal::create(
        FilesystemStudioJournalStore::new(directory.path()).expect("filesystem journal store"),
        prepared_render_snapshot(&ticket),
    )
    .expect("persist render reservation");
    let dispatch = journal
        .into_render_authorization()
        .expect("durable dispatch authorization")
        .bind(ticket)
        .expect("bind persisted ticket");
    let mut original = StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    assert_eq!(
        original.start(dispatch).expect("renderer start"),
        RenderSessionState::Running
    );
    let active = state
        .borrow()
        .active
        .get(&export_id(197))
        .cloned()
        .expect("active renderer");
    state.borrow_mut().postconditions.insert(
        export_id(197),
        RenderPostcondition::Committed {
            fence: active.fence,
            render_spec_digest: active.render_spec_digest,
            output_checksum: checksum(200),
            output_bytes: 54_321,
        },
    );
    let renderer = original.into_renderer();

    let reopened = DurableStudioJournal::open(
        FilesystemStudioJournalStore::new(directory.path()).expect("reopened filesystem store"),
        project,
    )
    .expect("reopen running journal");
    assert_eq!(reopened.snapshot().boundary, JournalBoundary::RenderRunning);
    assert!(
        reopened
            .snapshot()
            .pending_render
            .as_ref()
            .expect("pending render")
            .terminal_receipt
            .is_none()
    );
    let authorization = reopened
        .into_render_authorization()
        .expect("recovered authorization");
    let mut restarted = StudioRenderCoordinator::new(renderer, 8, vec![authorization])
        .expect("restart with durable reservation");
    assert_eq!(
        restarted
            .adopt_recovered_commit(export_id(197))
            .expect("journal then adopt exact commit"),
        RenderSessionState::Committed
    );

    let committed = DurableStudioJournal::open(
        FilesystemStudioJournalStore::new(directory.path()).expect("verification store"),
        project,
    )
    .expect("reopen committed journal");
    assert_eq!(
        committed.snapshot().boundary,
        JournalBoundary::RenderCommitted
    );
    let receipt = committed
        .snapshot()
        .pending_render
        .as_ref()
        .and_then(|pending| pending.terminal_receipt.as_ref())
        .expect("durable terminal receipt");
    assert_eq!(receipt.output_checksum, checksum(200));
    assert_eq!(receipt.output_bytes, 54_321);
}

#[test]
fn coordinator_restart_adopts_only_the_exact_committed_output() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let ticket = render_ticket_to(&capabilities, 186, 187, EncoderBackend::Hardware, 188);
    let reservation = committed_recovery_reservation(&ticket, checksum(190), 98_765);
    let mut original = StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    original.start(ticket).expect("original start");
    let active = state
        .borrow()
        .active
        .get(&export_id(186))
        .cloned()
        .expect("active render");
    state.borrow_mut().postconditions.insert(
        export_id(186),
        RenderPostcondition::Committed {
            fence: active.fence,
            render_spec_digest: active.render_spec_digest,
            output_checksum: checksum(191),
            output_bytes: 98_765,
        },
    );
    let renderer = original.into_renderer();
    let mut restarted = StudioRenderCoordinator::new(renderer, 8, vec![reservation])
        .expect("restart with journal reservation");
    assert_eq!(
        restarted.adopt_recovered_commit(export_id(186)),
        Err(StudioError::RenderPostconditionMismatch)
    );
    assert_eq!(
        restarted.start(render_ticket_to(
            &capabilities,
            192,
            193,
            EncoderBackend::Hardware,
            188,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    state.borrow_mut().postconditions.insert(
        export_id(186),
        RenderPostcondition::Committed {
            fence: active.fence,
            render_spec_digest: active.render_spec_digest,
            output_checksum: checksum(190),
            output_bytes: 98_765,
        },
    );
    assert_eq!(
        restarted
            .adopt_recovered_commit(export_id(186))
            .expect("adopt exact committed output"),
        RenderSessionState::Committed
    );
    assert!(restarted.receipt(operation_id(187)).is_some());
    restarted
        .release_terminal(export_id(186))
        .expect("release adopted commit");
}

#[test]
fn concurrent_export_identity_and_deadline_are_fenced_and_cleaned() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            86,
            87,
            EncoderBackend::Hardware,
        ))
        .expect("start");
    assert_eq!(
        coordinator
            .start(render_ticket(
                &capabilities,
                86,
                87,
                EncoderBackend::Hardware,
            ))
            .expect("exact active replay"),
        RenderSessionState::Running
    );
    assert_eq!(
        coordinator.start(render_ticket(
            &capabilities,
            89,
            87,
            EncoderBackend::Hardware,
        )),
        Err(StudioError::IdempotencyConflict)
    );
    assert_eq!(
        coordinator.start(render_ticket(
            &capabilities,
            86,
            88,
            EncoderBackend::Hardware,
        )),
        Err(StudioError::ExportIdReused)
    );
    assert_eq!(
        coordinator.enforce_deadline(
            export_id(86),
            Duration::from_secs(3_600),
            Duration::from_secs(1),
        ),
        Err(StudioError::RenderDeadlineExceeded)
    );
    assert_eq!(
        coordinator
            .release_terminal(export_id(86))
            .expect("release cancelled"),
        RenderSessionState::Cancelled
    );
    assert_eq!(
        coordinator.start(render_ticket(
            &capabilities,
            86,
            89,
            EncoderBackend::Hardware,
        )),
        Err(StudioError::ExportIdReused)
    );
    assert_eq!(
        state.borrow().postconditions.get(&export_id(86)).cloned(),
        Some(RenderPostcondition::Absent)
    );
}

#[test]
fn render_replay_binds_the_exact_sources_edits_profile_and_output() {
    let (renderer, _) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            124,
            125,
            EncoderBackend::Hardware,
        ))
        .expect("initial render");

    let mut changed_project = manifest();
    changed_project.id = project_id(80);
    changed_project.assets[0].checksum = checksum(126);
    let changed_sources = render_ticket_for_project(
        &capabilities,
        124,
        125,
        EncoderBackend::Hardware,
        124,
        changed_project,
        source(),
    );
    assert_eq!(
        coordinator.start(changed_sources),
        Err(StudioError::IdempotencyConflict)
    );

    let mut changed_edits = manifest();
    changed_edits.id = project_id(80);
    changed_edits.edits.operations.push(EditOperation::Split {
        at: RationalTime::new(13, TimeBase::new(2).expect("timebase")),
    });
    let changed_plan = render_ticket_for_project(
        &capabilities,
        124,
        125,
        EncoderBackend::Hardware,
        124,
        changed_edits,
        source(),
    );
    assert_eq!(
        coordinator.start(changed_plan),
        Err(StudioError::IdempotencyConflict)
    );

    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            124,
            125,
            EncoderBackend::Hardware,
            126,
        )),
        Err(StudioError::IdempotencyConflict)
    );

    let mut source_project = manifest();
    source_project.id = project_id(80);
    let timeline = source();
    let plan = StudioTimelineCompiler::compile(&timeline, &source_project.edits).expect("plan");
    let sources = StudioSourceSet::from_project(&source_project, &timeline).expect("sources");
    let preflight = preflight_render(ExportProfile::NativeHighQualityWebM, &capabilities)
        .expect("WebM preflight");
    let graph = StudioRenderGraphSpec::compile(sources, plan, preflight).expect("WebM graph");
    let changed_profile = StudioRenderTicket::new(
        project_id(80),
        export_id(124),
        operation_id(125),
        5,
        StudioSourceName::new("export-124.mp4").expect("output"),
        graph,
        Duration::from_secs(3_600),
    )
    .expect("changed-profile ticket");
    assert_eq!(
        coordinator.start(authorize_ticket(changed_profile)),
        Err(StudioError::IdempotencyConflict)
    );
}

#[test]
fn output_targets_are_portable_and_reserved_until_terminal_release() {
    for invalid in ["Export.mp4", "con.mp4", "name.", "two words.mp4"] {
        assert_eq!(
            StudioSourceName::new(invalid),
            Err(StudioError::InvalidSourceName),
            "{invalid} must not be a portable Studio output name"
        );
    }

    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket_to(
            &capabilities,
            127,
            128,
            EncoderBackend::Hardware,
            129,
        ))
        .expect("reserve output");
    assert_eq!(
        coordinator.start(render_ticket_to(
            &capabilities,
            130,
            131,
            EncoderBackend::Hardware,
            129,
        )),
        Err(StudioError::OutputTargetBusy)
    );
    coordinator
        .cancel_and_cleanup(export_id(127), Duration::from_secs(1))
        .expect("clean first output");
    coordinator
        .release_terminal(export_id(127))
        .expect("release first output");
    assert_eq!(
        coordinator
            .start(render_ticket_to(
                &capabilities,
                130,
                131,
                EncoderBackend::Hardware,
                129,
            ))
            .expect("reuse released output"),
        RenderSessionState::Running
    );
    assert_eq!(state.borrow().cleanups.len(), 1);
}

#[test]
fn render_progress_and_failures_are_observable_through_bounded_events() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 2, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            132,
            133,
            EncoderBackend::Hardware,
        ))
        .expect("start");
    assert_eq!(
        coordinator
            .progress(export_id(132))
            .expect("initial progress"),
        RenderProgressSnapshot {
            state: RenderSessionState::Running,
            phase: None,
            basis_points: 0,
            last_sequence: 0,
            failure_code: None,
        }
    );

    push_event(
        &state,
        132,
        1,
        5,
        RenderEventKind::Progress {
            phase: RenderPhase::Encoding,
            basis_points: 2_500,
        },
    );
    coordinator
        .poll(export_id(132), Duration::from_millis(1))
        .expect("progress event");
    assert_eq!(
        coordinator.progress(export_id(132)).expect("progress"),
        RenderProgressSnapshot {
            state: RenderSessionState::Running,
            phase: Some(RenderPhase::Encoding),
            basis_points: 2_500,
            last_sequence: 1,
            failure_code: None,
        }
    );
    let drained = coordinator
        .drain_events(export_id(132), 1)
        .expect("bounded drain");
    assert_eq!(drained.len(), 1);
    assert!(matches!(
        drained[0].kind,
        RenderEventKind::Progress {
            phase: RenderPhase::Encoding,
            basis_points: 2_500
        }
    ));
    assert_eq!(
        coordinator.drain_events(export_id(132), 0),
        Err(StudioError::UnboundedRendererEvents)
    );

    push_event(
        &state,
        132,
        2,
        5,
        RenderEventKind::Failed {
            safe_code: "encoder-unavailable",
            hardware_failure: false,
        },
    );
    assert_eq!(
        coordinator
            .poll(export_id(132), Duration::from_millis(1))
            .expect("failure event"),
        RenderSessionState::Failed
    );
    assert_eq!(
        coordinator
            .progress(export_id(132))
            .expect("failed progress"),
        RenderProgressSnapshot {
            state: RenderSessionState::Failed,
            phase: Some(RenderPhase::Encoding),
            basis_points: 2_500,
            last_sequence: 2,
            failure_code: Some("encoder-unavailable"),
        }
    );
    assert_eq!(state.borrow().cleanups.len(), 1);
}

#[test]
fn render_progress_rejects_phase_regression_and_unsafe_failure_codes() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            134,
            135,
            EncoderBackend::Hardware,
        ))
        .expect("start");
    push_event(
        &state,
        134,
        1,
        5,
        RenderEventKind::Progress {
            phase: RenderPhase::Encoding,
            basis_points: 2_500,
        },
    );
    coordinator
        .poll(export_id(134), Duration::from_millis(1))
        .expect("encoding progress");
    push_event(
        &state,
        134,
        2,
        5,
        RenderEventKind::Progress {
            phase: RenderPhase::Decoding,
            basis_points: 3_000,
        },
    );
    assert_eq!(
        coordinator.poll(export_id(134), Duration::from_millis(1)),
        Err(StudioError::NonMonotonicProgress)
    );

    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            136,
            137,
            EncoderBackend::Hardware,
        ))
        .expect("start");
    push_event(
        &state,
        136,
        1,
        5,
        RenderEventKind::Failed {
            safe_code: "Private Internal Error",
            hardware_failure: false,
        },
    );
    assert_eq!(
        coordinator.poll(export_id(136), Duration::from_millis(1)),
        Err(StudioError::InvalidRenderFailureCode)
    );
}

#[test]
fn stale_callbacks_are_rejected_and_each_render_phase_cleans_exact_partial() {
    let phases = [
        RenderPhase::Preparing,
        RenderPhase::Decoding,
        RenderPhase::Compositing,
        RenderPhase::Encoding,
        RenderPhase::Muxing,
        RenderPhase::Finalizing,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let marker = 90_u8.saturating_add(u8::try_from(index).expect("phase index"));
        let (renderer, state) = FakeRenderer::new();
        let capabilities = renderer.capabilities.clone();
        let mut coordinator =
            StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
        coordinator
            .start(render_ticket(
                &capabilities,
                marker,
                marker.saturating_add(20),
                EncoderBackend::Hardware,
            ))
            .expect("start");
        push_event(
            &state,
            marker,
            1,
            5,
            RenderEventKind::Progress {
                phase,
                basis_points: 100,
            },
        );
        assert_eq!(
            coordinator
                .poll(export_id(marker), Duration::from_millis(1))
                .expect("progress"),
            RenderSessionState::Running
        );
        coordinator
            .cancel_and_cleanup(export_id(marker), Duration::from_secs(1))
            .expect("cancel and clean");
        assert_eq!(
            state
                .borrow()
                .postconditions
                .get(&export_id(marker))
                .cloned(),
            Some(RenderPostcondition::Absent)
        );
        let cleanup = state.borrow().cleanups.last().cloned().expect("cleanup");
        assert_eq!(cleanup.0, export_id(marker));
        assert_eq!(cleanup.1, 5);
        assert_eq!(
            cleanup.2,
            state
                .borrow()
                .active
                .get(&export_id(marker))
                .expect("active render")
                .render_spec_digest
        );
        assert_eq!(cleanup.3.as_str(), format!("export-{marker}.mp4").as_str());
    }

    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            110,
            111,
            EncoderBackend::Hardware,
        ))
        .expect("start");
    push_event(
        &state,
        110,
        1,
        6,
        RenderEventKind::Progress {
            phase: RenderPhase::Encoding,
            basis_points: 500,
        },
    );
    assert_eq!(
        coordinator.poll(export_id(110), Duration::from_millis(1)),
        Err(StudioError::StaleRenderCallback)
    );
}

#[test]
fn hardware_failure_cleans_then_restarts_the_identical_plan_in_software() {
    let (renderer, state) = FakeRenderer::new();
    let capabilities = renderer.capabilities.clone();
    let mut coordinator =
        StudioRenderCoordinator::new(renderer, 8, Vec::new()).expect("coordinator");
    coordinator
        .start(render_ticket(
            &capabilities,
            120,
            121,
            EncoderBackend::Hardware,
        ))
        .expect("hardware start");
    push_event(
        &state,
        120,
        1,
        5,
        RenderEventKind::Failed {
            safe_code: "hardware-encoder-failed",
            hardware_failure: true,
        },
    );
    assert_eq!(
        coordinator
            .poll(export_id(120), Duration::from_millis(1))
            .expect("failure"),
        RenderSessionState::Failed
    );
    assert_eq!(
        coordinator.retry_hardware_failure_with_software(
            export_id(120),
            Duration::from_secs(1),
            render_ticket_to(&capabilities, 122, 123, EncoderBackend::Software, 121),
        ),
        Err(StudioError::InvalidHardwareFallback)
    );
    assert_eq!(
        coordinator
            .retry_hardware_failure_with_software(
                export_id(120),
                Duration::from_secs(1),
                render_ticket_to(&capabilities, 122, 123, EncoderBackend::Software, 120),
            )
            .expect("software fallback"),
        RenderSessionState::Running
    );
    assert_eq!(state.borrow().cleanups.len(), 1);
}

#[test]
fn debug_output_redacts_project_asset_and_render_identifiers() {
    let project = project_id(200);
    let media = asset(201, TrackKind::Screen);
    let (renderer, _) = FakeRenderer::new();
    let ticket = render_ticket(&renderer.capabilities, 202, 203, EncoderBackend::Hardware);
    let rendered = format!("{project:?} {media:?} {ticket:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("201201"));
    assert!(!rendered.contains(media.source_name.as_str()));
}
