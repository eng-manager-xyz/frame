use std::collections::BTreeSet;

use frame_media::{
    AudioCodec, ConformanceBudget, ConformanceError, ConformanceFailure, ExecutionFailureClass,
    FaultExpectation, FaultScenario, FuzzEnvelopeKind, LatencyBudget, LatencyFailure,
    LogicalResultObservation, ManagedMediaContract, ManagedOutputMode, MediaCapabilityRouter,
    MediaExecutorKind, MediaInput, MediaInputRole, MediaJobKind, MediaOutputFormat,
    MediaProbeSummary, MediaServiceError, MediaServiceRouteReason, MediaTransformProfile,
    NativeMediaContract, PairMeasurements, PrivateMediaInput, ProbeTrust, ProgressSample,
    RecoveryDisposition, RegressionGate, RegressionObservation, RegressionSeed, ResizeFit,
    ResourceBudget, ResourceFailure, ResourceSample, RoutingObservation, RoutingResult, VideoCodec,
    compare_media, evaluate_latency_distribution, evaluate_resource_trace, media_service_catalog,
    validate_conformance_fuzz_envelope, validate_progress_trace, verify_fault_matrix,
    verify_routing_observation, verify_seeded_regressions, verify_single_logical_result,
};
use serde::Deserialize;
use serde_json::Value;

const OFFLINE_CASES: &str =
    include_str!("../../../fixtures/media-conformance/v1/offline-cases.json");
const MATRIX: &str = include_str!("../../../fixtures/media-conformance/v1/matrix.json");
const PROTECTED: &str = include_str!("../../../fixtures/media-conformance/v1/protected-lanes.json");
const FUZZ_CORPUS: &str = include_str!("../../../fixtures/media-conformance/v1/fuzz-corpus.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineSuite {
    schema_version: u16,
    suite_id: String,
    seed: String,
    media_pairs: Vec<MediaPairCase>,
    progress_traces: Vec<ProgressCase>,
    resource_traces: Vec<ResourceCase>,
    latency_distributions: Vec<LatencyCase>,
    fault_matrix: Vec<(String, String)>,
    routing_observations: Vec<(String, String, u16, u16, u16, String)>,
    logical_results: Vec<LogicalCase>,
    regression_gates: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaPairCase {
    id: String,
    reference: ProbeFixture,
    candidate: ProbeFixture,
    measurements: MeasurementFixture,
    budget: BudgetFixture,
    expected_failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeFixture {
    container: String,
    video_codec: String,
    audio_codec: String,
    width: u32,
    height: u32,
    duration_ms: u64,
    frame_rate_milli: u32,
    av_offset_ms: i64,
    playable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementFixture {
    perceptual_milli: u16,
    waveform_correlation_milli: u16,
    max_caption_timing_delta_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetFixture {
    duration_tolerance_ms: u64,
    frame_rate_tolerance_milli: u32,
    av_offset_tolerance_ms: u64,
    minimum_perceptual_milli: u16,
    minimum_waveform_correlation_milli: u16,
    caption_timing_tolerance_ms: u64,
    require_same_dimensions: bool,
    require_same_container: bool,
    require_same_codecs: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressCase {
    id: String,
    require_complete: bool,
    samples: Vec<(u64, Option<u16>)>,
    expected: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceCase {
    id: String,
    samples: Vec<Vec<u64>>,
    budget: Vec<u64>,
    expected_failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LatencyCase {
    id: String,
    samples_ms: Vec<u64>,
    budget: Vec<u64>,
    expected_failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalCase {
    id: String,
    require_publication: bool,
    observations: Vec<(String, String, u16, u16)>,
    expected: String,
}

fn suite() -> OfflineSuite {
    serde_json::from_str(OFFLINE_CASES).expect("strict offline conformance fixture")
}

fn probe(fixture: &ProbeFixture) -> MediaProbeSummary {
    MediaProbeSummary {
        container: match fixture.container.as_str() {
            "mp4" => frame_media::ContainerFormat::Mp4,
            "quicktime" => frame_media::ContainerFormat::QuickTime,
            "webm" => frame_media::ContainerFormat::WebM,
            value => panic!("unsupported fixture container {value}"),
        },
        video_codec: match fixture.video_codec.as_str() {
            "h264" => VideoCodec::H264,
            "vp8" => VideoCodec::Vp8,
            value => panic!("unsupported fixture video codec {value}"),
        },
        audio_codec: match fixture.audio_codec.as_str() {
            "aac" => AudioCodec::Aac,
            "opus" => AudioCodec::Opus,
            value => panic!("unsupported fixture audio codec {value}"),
        },
        width: fixture.width,
        height: fixture.height,
        duration_ms: fixture.duration_ms,
        frame_rate_milli: fixture.frame_rate_milli,
        av_offset_ms: fixture.av_offset_ms,
        playable: fixture.playable,
    }
}

fn failure_id(failure: ConformanceFailure) -> &'static str {
    match failure {
        ConformanceFailure::NotPlayable => "not_playable",
        ConformanceFailure::ContainerMismatch => "container",
        ConformanceFailure::CodecMismatch => "codec",
        ConformanceFailure::DimensionMismatch => "dimension",
        ConformanceFailure::DurationDelta { .. } => "duration",
        ConformanceFailure::FrameRateDelta { .. } => "frame_rate",
        ConformanceFailure::AvOffset { .. } => "av_sync",
        ConformanceFailure::PerceptualScore { .. } => "perceptual",
        ConformanceFailure::WaveformScore { .. } => "waveform",
        ConformanceFailure::CaptionTiming { .. } => "caption_timing",
    }
}

#[test]
fn immutable_offline_corpus_drives_every_comparator() {
    let suite = suite();
    assert_eq!(suite.schema_version, 1);
    assert_eq!(suite.suite_id, "media-conformance-offline-v1");
    assert_eq!(suite.seed, "frame-media-conformance-20260716");

    for case in suite.media_pairs {
        assert!(!case.id.is_empty());
        let report = compare_media(
            probe(&case.reference),
            probe(&case.candidate),
            PairMeasurements {
                perceptual_milli: case.measurements.perceptual_milli,
                waveform_correlation_milli: case.measurements.waveform_correlation_milli,
                max_caption_timing_delta_ms: case.measurements.max_caption_timing_delta_ms,
            },
            ConformanceBudget {
                duration_tolerance_ms: case.budget.duration_tolerance_ms,
                frame_rate_tolerance_milli: case.budget.frame_rate_tolerance_milli,
                av_offset_tolerance_ms: case.budget.av_offset_tolerance_ms,
                minimum_perceptual_milli: case.budget.minimum_perceptual_milli,
                minimum_waveform_correlation_milli: case.budget.minimum_waveform_correlation_milli,
                caption_timing_tolerance_ms: case.budget.caption_timing_tolerance_ms,
                require_same_dimensions: case.budget.require_same_dimensions,
                require_same_container: case.budget.require_same_container,
                require_same_codecs: case.budget.require_same_codecs,
            },
        )
        .expect("valid comparison");
        assert_eq!(
            report
                .failures
                .into_iter()
                .map(failure_id)
                .collect::<Vec<_>>(),
            case.expected_failures,
            "{}",
            case.id
        );
    }

    for case in suite.progress_traces {
        let samples: Vec<_> = case
            .samples
            .into_iter()
            .map(|(elapsed_ms, basis_points)| ProgressSample {
                elapsed_ms,
                basis_points,
            })
            .collect();
        let result = validate_progress_trace(&samples, case.require_complete);
        match case.expected.as_str() {
            "pass" => result.expect("valid progress trace"),
            "non_monotonic_progress" => assert_eq!(
                result,
                Err(ConformanceError::NonMonotonicProgress),
                "{}",
                case.id
            ),
            value => panic!("unknown progress expectation {value}"),
        }
    }

    for case in suite.resource_traces {
        assert_eq!(case.budget.len(), 12, "{}", case.id);
        let samples: Vec<_> = case
            .samples
            .iter()
            .map(|sample| {
                assert_eq!(sample.len(), 11, "{}", case.id);
                ResourceSample {
                    elapsed_ms: sample[0],
                    memory_bytes: sample[1],
                    handles: sample[2],
                    threads: u32::try_from(sample[3]).expect("threads fit"),
                    disk_bytes: sample[4],
                    cpu_milli_percent: u32::try_from(sample[5]).expect("CPU fit"),
                    gpu_milli_percent: u32::try_from(sample[6]).expect("GPU fit"),
                    temperature_milli_celsius: u32::try_from(sample[7]).expect("temperature fit"),
                    output_latency_ms: sample[8],
                    av_drift_ms: i64::try_from(sample[9]).expect("drift fit"),
                    cost_microunits: sample[10],
                }
            })
            .collect();
        let failures = evaluate_resource_trace(
            &samples,
            ResourceBudget {
                max_memory_bytes: case.budget[0],
                max_memory_growth_bytes: case.budget[1],
                max_handles: case.budget[2],
                max_handle_growth: case.budget[3],
                max_threads: u32::try_from(case.budget[4]).expect("thread budget fit"),
                max_disk_bytes: case.budget[5],
                max_cpu_milli_percent: u32::try_from(case.budget[6]).expect("CPU budget fit"),
                max_gpu_milli_percent: u32::try_from(case.budget[7]).expect("GPU budget fit"),
                max_temperature_milli_celsius: u32::try_from(case.budget[8])
                    .expect("temperature budget fit"),
                max_output_latency_ms: case.budget[9],
                max_av_drift_ms: case.budget[10],
                max_cost_microunits: case.budget[11],
            },
        )
        .expect("valid resource trace");
        assert_eq!(
            failures.into_iter().map(resource_id).collect::<Vec<_>>(),
            case.expected_failures,
            "{}",
            case.id
        );
    }

    for case in suite.latency_distributions {
        assert_eq!(case.budget.len(), 4, "{}", case.id);
        let failures = evaluate_latency_distribution(
            &case.samples_ms,
            LatencyBudget {
                p50_ms: case.budget[0],
                p95_ms: case.budget[1],
                p99_ms: case.budget[2],
                maximum_ms: case.budget[3],
            },
        )
        .expect("valid latency distribution");
        assert_eq!(
            failures.into_iter().map(latency_id).collect::<Vec<_>>(),
            case.expected_failures,
            "{}",
            case.id
        );
    }
}

fn resource_id(failure: ResourceFailure) -> &'static str {
    match failure {
        ResourceFailure::MemoryLimit => "memory_limit",
        ResourceFailure::MemoryGrowth => "memory_growth",
        ResourceFailure::HandleLimit => "handle_limit",
        ResourceFailure::HandleGrowth => "handle_growth",
        ResourceFailure::ThreadLimit => "thread_limit",
        ResourceFailure::DiskLimit => "disk_limit",
        ResourceFailure::CpuLimit => "cpu_limit",
        ResourceFailure::GpuLimit => "gpu_limit",
        ResourceFailure::TemperatureLimit => "temperature_limit",
        ResourceFailure::LatencyLimit => "latency_limit",
        ResourceFailure::AvDrift => "av_drift",
        ResourceFailure::CostLimit => "cost_limit",
    }
}

fn latency_id(failure: LatencyFailure) -> &'static str {
    match failure {
        LatencyFailure::P50 => "p50",
        LatencyFailure::P95 => "p95",
        LatencyFailure::P99 => "p99",
        LatencyFailure::Maximum => "maximum",
    }
}

#[test]
fn fault_routing_idempotency_and_regression_corpora_fail_the_declared_gate() {
    let suite = suite();
    let fault_matrix: Vec<_> = suite
        .fault_matrix
        .iter()
        .map(|(scenario, disposition)| FaultExpectation {
            scenario: fault_scenario(scenario),
            disposition: recovery_disposition(disposition),
        })
        .collect();
    verify_fault_matrix(&fault_matrix, &fault_matrix).expect("complete fault matrix");
    assert_eq!(fault_matrix.len(), 13);

    for (id, result, managed, native, external, expected) in suite.routing_observations {
        let result = routing_result(&result);
        let actual = verify_routing_observation(
            result,
            RoutingObservation {
                result,
                managed_invocations: managed,
                native_invocations: native,
                external_invocations: external,
            },
        );
        match expected.as_str() {
            "pass" => actual.expect("valid route observation"),
            "unexpected_invocation" => {
                assert_eq!(actual, Err(ConformanceError::UnexpectedInvocation), "{id}")
            }
            value => panic!("unknown routing expectation {value}"),
        }
    }

    for case in suite.logical_results {
        let observations: Vec<_> = case
            .observations
            .iter()
            .map(
                |(logical_key, checksum_sha256, publication_effects, billable_effects)| {
                    LogicalResultObservation {
                        logical_key,
                        checksum_sha256,
                        publication_effects: *publication_effects,
                        billable_effects: *billable_effects,
                    }
                },
            )
            .collect();
        let result = verify_single_logical_result(&observations, case.require_publication);
        match case.expected.as_str() {
            "pass" => {
                result.expect("valid logical result");
            }
            "duplicate_logical_effect" => assert_eq!(
                result,
                Err(ConformanceError::DuplicateLogicalEffect),
                "{}",
                case.id
            ),
            value => panic!("unknown logical result expectation {value}"),
        }
    }

    let regressions: Vec<_> = suite
        .regression_gates
        .iter()
        .map(|(seed, gate)| RegressionObservation {
            seed: regression_seed(seed),
            failed_gate: regression_gate(gate),
        })
        .collect();
    verify_seeded_regressions(&regressions).expect("complete seeded regression matrix");
}

#[test]
fn managed_boundaries_route_exactly_and_just_over_without_remote_execution() {
    let router = router();
    let cases = [
        (
            99_999_999,
            600_000,
            10,
            1_000,
            MediaExecutorKind::CloudflareMedia,
        ),
        (
            100_000_000,
            600_000,
            10,
            1_000,
            MediaExecutorKind::NativeGstreamer,
        ),
        (
            99_999_999,
            600_001,
            10,
            1_000,
            MediaExecutorKind::NativeGstreamer,
        ),
        (
            99_999_999,
            600_000,
            9,
            1_000,
            MediaExecutorKind::NativeGstreamer,
        ),
        (
            99_999_999,
            600_000,
            2_000,
            60_000,
            MediaExecutorKind::CloudflareMedia,
        ),
        (
            99_999_999,
            600_000,
            2_001,
            60_000,
            MediaExecutorKind::NativeGstreamer,
        ),
        (
            99_999_999,
            600_000,
            10,
            999,
            MediaExecutorKind::NativeGstreamer,
        ),
        (
            99_999_999,
            600_000,
            10,
            60_001,
            MediaExecutorKind::NativeGstreamer,
        ),
    ];
    for (bytes, duration_ms, dimension, output_duration_ms, executor) in cases {
        let input = private_input(bytes, duration_ms, VideoCodec::H264, AudioCodec::Aac);
        let profile = profile(dimension, output_duration_ms);
        let decision = router
            .route(MediaJobKind::OptimizedClip, &input, Some(&profile))
            .expect("bounded route");
        assert_eq!(decision.executor, executor);
        if executor == MediaExecutorKind::CloudflareMedia {
            assert_eq!(decision.reason, MediaServiceRouteReason::ManagedPreferred);
        } else {
            assert!(matches!(
                decision.reason,
                MediaServiceRouteReason::ManagedInputLimit
                    | MediaServiceRouteReason::ManagedProfileLimit
            ));
        }
    }

    let input = private_input(99_999_999, 600_000, VideoCodec::H264, AudioCodec::Aac);
    let profile = profile(1_920, 60_000);
    let managed = router
        .route(MediaJobKind::OptimizedClip, &input, Some(&profile))
        .expect("managed route");
    for failure in [
        ExecutionFailureClass::Quota,
        ExecutionFailureClass::Timeout,
        ExecutionFailureClass::ProviderOutage,
        ExecutionFailureClass::OutputIncompatible,
        ExecutionFailureClass::BetaRegression,
    ] {
        let fallback = router
            .fallback_after_failure(managed, failure, &input, &profile)
            .expect("declared native fallback");
        assert_eq!(fallback.executor, MediaExecutorKind::NativeGstreamer);
        assert_eq!(
            fallback.reason,
            MediaServiceRouteReason::ManagedFailure(failure)
        );
    }
    assert_eq!(
        router.fallback_after_failure(
            managed,
            ExecutionFailureClass::SecurityViolation,
            &input,
            &profile
        ),
        Err(MediaServiceError::FallbackForbidden)
    );
}

#[test]
fn matrix_has_complete_dimensions_and_protected_records_remain_uncollected() {
    let matrix: Value = serde_json::from_str(MATRIX).expect("matrix JSON");
    assert_eq!(matrix["schema_version"], 1);
    let cases = matrix["cases"].as_array().expect("matrix cases");
    assert!(cases.len() >= 40);
    let case_ids: BTreeSet<_> = cases
        .iter()
        .map(|case| case["id"].as_str().expect("case id"))
        .collect();
    assert_eq!(case_ids.len(), cases.len());
    for dimension in [
        "platforms",
        "sources",
        "modes",
        "video_codecs",
        "audio_codecs",
        "containers",
        "resolutions",
        "executors",
        "capability_boundaries",
        "faults",
    ] {
        assert!(
            matrix["dimensions"][dimension]
                .as_array()
                .is_some_and(|values| !values.is_empty()),
            "missing {dimension}"
        );
    }

    let protected: Value = serde_json::from_str(PROTECTED).expect("protected plan JSON");
    let records = protected["records"].as_array().expect("protected records");
    assert_eq!(records.len(), 7);
    for record in records {
        assert_eq!(record["status"], "not_collected");
        assert!(record["evidence_sha256"].is_null());
        assert!(record["observed_at"].is_null());
        assert!(
            record["required_cases"]
                .as_array()
                .is_some_and(|v| !v.is_empty())
        );
    }
}

#[test]
fn deterministic_fuzz_corpus_and_byte_mutations_never_escape_the_parser() {
    let corpus: Value = serde_json::from_str(FUZZ_CORPUS).expect("fuzz corpus JSON");
    assert_eq!(corpus["schema_version"], 1);
    assert_eq!(corpus["deterministic_seed"], 290_429);
    let cases = corpus["cases"].as_array().expect("fuzz cases");
    assert!(cases.len() >= 10);
    let mut mutation_count = 0_u64;
    for case in cases {
        let id = case["id"].as_str().expect("fuzz case ID");
        let encoded = case["hex"].as_str().expect("fuzz case hex");
        let expected = case["expected"].as_str().expect("fuzz expectation");
        let bytes = decode_hex(encoded);
        let result = validate_conformance_fuzz_envelope(&bytes);
        match expected {
            "invalid" => assert_eq!(result, Err(ConformanceError::InvalidFuzzEnvelope), "{id}"),
            "valid_label" => assert_eq!(result, Ok(FuzzEnvelopeKind::Label), "{id}"),
            "valid_object_key" => assert_eq!(result, Ok(FuzzEnvelopeKind::ObjectKey), "{id}"),
            "valid_sha256" => assert_eq!(result, Ok(FuzzEnvelopeKind::Sha256), "{id}"),
            "valid_progress" => assert_eq!(result, Ok(FuzzEnvelopeKind::Progress), "{id}"),
            value => panic!("unknown fuzz expectation {value}"),
        }

        for index in 0..bytes.len() {
            for mask in [1_u8, 2, 4, 0x7f, 0x80, 0xff] {
                let mut mutated = bytes.clone();
                mutated[index] ^= mask;
                let _ = validate_conformance_fuzz_envelope(&mutated);
                mutation_count = mutation_count.saturating_add(1);
            }
            let _ = validate_conformance_fuzz_envelope(&bytes[..index]);
            mutation_count = mutation_count.saturating_add(1);
        }
    }
    assert!(mutation_count >= 1_000, "fuzz campaign unexpectedly small");
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2));
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(text, 16).expect("valid hex")
        })
        .collect()
}

fn router() -> MediaCapabilityRouter {
    let spec = media_service_catalog()
        .get(MediaJobKind::OptimizedClip)
        .expect("optimized clip spec");
    MediaCapabilityRouter::new(
        true,
        ManagedMediaContract::cloudflare_2026_06_10(),
        NativeMediaContract {
            limits: spec.sandbox,
            max_decompression_ratio: 1_000,
        },
    )
    .expect("router")
}

fn private_input(
    bytes: u64,
    duration_ms: u64,
    video_codec: VideoCodec,
    audio_codec: AudioCodec,
) -> PrivateMediaInput {
    PrivateMediaInput {
        tenant_id: "11111111-1111-4111-8111-111111111111".into(),
        video_id: "22222222-2222-4222-8222-222222222222".into(),
        object_key: "tenants/11111111-1111-4111-8111-111111111111/videos/22222222-2222-4222-8222-222222222222/source/v1.mp4".into(),
        role: MediaInputRole::SourceOriginal,
        source_version: 1,
        source_sha256: "a".repeat(64),
        metadata: MediaInput {
            bytes,
            duration_ms,
            width: 1_920,
            height: 1_080,
            container: frame_media::ContainerFormat::Mp4,
            video_codec,
            audio_codec,
            encrypted: false,
        },
        decoded_bytes_upper_bound: bytes.saturating_mul(2),
        frame_count_upper_bound: 18_001,
        track_count: 2,
        probe_trust: ProbeTrust::VerifiedNativeProbe,
    }
}

fn profile(dimension: u32, duration_ms: u64) -> MediaTransformProfile {
    MediaTransformProfile {
        schema_version: 1,
        profile_id: "optimized_clip_v1".into(),
        profile_version: 1,
        mode: ManagedOutputMode::Video,
        start_ms: 0,
        duration_ms: Some(duration_ms),
        width: Some(dimension),
        height: Some(dimension),
        fit: ResizeFit::Contain,
        image_count: None,
        include_audio: true,
        format: MediaOutputFormat::Mp4H264Aac,
        max_output_bytes: 32_000_000,
    }
}

fn fault_scenario(value: &str) -> FaultScenario {
    match value {
        "device_lost" => FaultScenario::DeviceLost,
        "permission_denied" => FaultScenario::PermissionDenied,
        "process_crash" => FaultScenario::ProcessCrash,
        "disk_full" => FaultScenario::DiskFull,
        "network_lost" => FaultScenario::NetworkLost,
        "provider_error" => FaultScenario::ProviderError,
        "provider_outage" => FaultScenario::ProviderOutage,
        "provider_quota" => FaultScenario::ProviderQuota,
        "managed_output_drift" => FaultScenario::ManagedOutputDrift,
        "cancellation" => FaultScenario::Cancellation,
        "unsupported_codec" => FaultScenario::UnsupportedCodec,
        "hardware_encoder_failure" => FaultScenario::HardwareEncoderFailure,
        "timeout" => FaultScenario::Timeout,
        other => panic!("unknown fault scenario {other}"),
    }
}

fn recovery_disposition(value: &str) -> RecoveryDisposition {
    match value {
        "resume" => RecoveryDisposition::Resume,
        "recover_artifact" => RecoveryDisposition::RecoverArtifact,
        "fail_actionable" => RecoveryDisposition::FailActionable,
        "fallback" => RecoveryDisposition::Fallback,
        "suppress_publication" => RecoveryDisposition::SuppressPublication,
        other => panic!("unknown recovery disposition {other}"),
    }
}

fn routing_result(value: &str) -> RoutingResult {
    match value {
        "managed" => RoutingResult::Managed,
        "native" => RoutingResult::Native,
        "external_provider" => RoutingResult::ExternalProvider,
        "rejected_before_invocation" => RoutingResult::RejectedBeforeInvocation,
        other => panic!("unknown routing result {other}"),
    }
}

fn regression_seed(value: &str) -> RegressionSeed {
    match value {
        "quality" => RegressionSeed::Quality,
        "sync" => RegressionSeed::Sync,
        "recovery" => RegressionSeed::Recovery,
        "resource" => RegressionSeed::Resource,
        "routing" => RegressionSeed::Routing,
        "provider_limit" => RegressionSeed::ProviderLimit,
        "managed_output_drift" => RegressionSeed::ManagedOutputDrift,
        "fallback" => RegressionSeed::Fallback,
        "repeat_cost" => RegressionSeed::RepeatCost,
        other => panic!("unknown regression seed {other}"),
    }
}

fn regression_gate(value: &str) -> RegressionGate {
    match value {
        "perceptual" => RegressionGate::Perceptual,
        "av_sync" => RegressionGate::AvSync,
        "recovery_state" => RegressionGate::RecoveryState,
        "resource_trend" => RegressionGate::ResourceTrend,
        "executor_routing" => RegressionGate::ExecutorRouting,
        "provider_preflight" => RegressionGate::ProviderPreflight,
        "output_compatibility" => RegressionGate::OutputCompatibility,
        "fallback_policy" => RegressionGate::FallbackPolicy,
        "cost_idempotency" => RegressionGate::CostIdempotency,
        other => panic!("unknown regression gate {other}"),
    }
}
