use thiserror::Error;

use crate::{AudioCodec, ContainerFormat, MediaExecutorKind, VideoCodec};

pub const CONFORMANCE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceLane {
    OfflineSynthetic,
    NativeHardware,
    RemoteManaged,
    ExternalProvider,
    CrossExecutor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactProvenance {
    pub schema_version: u16,
    pub fixture_id: String,
    pub fixture_sha256: String,
    pub source_revision: String,
    pub tool_version: String,
    pub runtime_version: String,
    pub os_family: String,
    pub architecture: String,
    pub hardware_class: Option<String>,
    pub provider_revision: Option<String>,
    pub profile_version: u16,
    pub executor: MediaExecutorKind,
    pub lane: EvidenceLane,
}

impl ArtifactProvenance {
    pub fn validate(self) -> Result<Self, ConformanceError> {
        if self.schema_version != CONFORMANCE_SCHEMA_VERSION
            || !safe_label(&self.fixture_id)
            || !lower_hex(&self.fixture_sha256, 64)
            || !lower_hex(&self.source_revision, 40)
            || !safe_label(&self.tool_version)
            || !safe_label(&self.runtime_version)
            || !safe_label(&self.os_family)
            || !safe_label(&self.architecture)
            || self
                .hardware_class
                .as_deref()
                .is_some_and(|value| !safe_label(value))
            || self
                .provider_revision
                .as_deref()
                .is_some_and(|value| !safe_label(value))
            || self.profile_version == 0
        {
            return Err(ConformanceError::InvalidProvenance);
        }
        let boundary_is_valid = match self.lane {
            EvidenceLane::OfflineSynthetic => {
                self.hardware_class.is_none() && self.provider_revision.is_none()
            }
            EvidenceLane::NativeHardware => {
                self.executor == MediaExecutorKind::NativeGstreamer
                    && self.hardware_class.is_some()
                    && self.provider_revision.is_none()
            }
            EvidenceLane::RemoteManaged => {
                self.executor == MediaExecutorKind::CloudflareMedia
                    && self.hardware_class.is_none()
                    && self.provider_revision.is_some()
            }
            EvidenceLane::ExternalProvider => {
                self.executor == MediaExecutorKind::ExternalProvider
                    && self.hardware_class.is_none()
                    && self.provider_revision.is_some()
            }
            EvidenceLane::CrossExecutor => {
                self.executor == MediaExecutorKind::ControlPlane
                    && self.hardware_class.is_some()
                    && self.provider_revision.is_some()
            }
        };
        if !boundary_is_valid {
            return Err(ConformanceError::InvalidProvenance);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaProbeSummary {
    pub container: ContainerFormat,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub frame_rate_milli: u32,
    pub av_offset_ms: i64,
    pub playable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairMeasurements {
    /// 0–1000 normalized full-reference perceptual score.
    pub perceptual_milli: u16,
    /// 0–1000 normalized waveform correlation.
    pub waveform_correlation_milli: u16,
    pub max_caption_timing_delta_ms: u64,
}

impl PairMeasurements {
    pub fn validate(self) -> Result<Self, ConformanceError> {
        if self.perceptual_milli > 1_000 || self.waveform_correlation_milli > 1_000 {
            return Err(ConformanceError::InvalidMeasurement);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConformanceBudget {
    pub duration_tolerance_ms: u64,
    pub frame_rate_tolerance_milli: u32,
    pub av_offset_tolerance_ms: u64,
    pub minimum_perceptual_milli: u16,
    pub minimum_waveform_correlation_milli: u16,
    pub caption_timing_tolerance_ms: u64,
    pub require_same_dimensions: bool,
    pub require_same_container: bool,
    pub require_same_codecs: bool,
}

impl ConformanceBudget {
    pub fn validate(self) -> Result<Self, ConformanceError> {
        if self.minimum_perceptual_milli > 1_000 || self.minimum_waveform_correlation_milli > 1_000
        {
            return Err(ConformanceError::InvalidBudget);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceFailure {
    NotPlayable,
    ContainerMismatch,
    CodecMismatch,
    DimensionMismatch,
    DurationDelta {
        actual_ms: u64,
        allowed_ms: u64,
    },
    FrameRateDelta {
        actual_milli: u32,
        allowed_milli: u32,
    },
    AvOffset {
        actual_ms: u64,
        allowed_ms: u64,
    },
    PerceptualScore {
        actual: u16,
        minimum: u16,
    },
    WaveformScore {
        actual: u16,
        minimum: u16,
    },
    CaptionTiming {
        actual_ms: u64,
        allowed_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceReport {
    pub failures: Vec<ConformanceFailure>,
}

impl ConformanceReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn compare_media(
    reference: MediaProbeSummary,
    candidate: MediaProbeSummary,
    measurements: PairMeasurements,
    budget: ConformanceBudget,
) -> Result<ConformanceReport, ConformanceError> {
    let measurements = measurements.validate()?;
    let budget = budget.validate()?;
    let mut failures = Vec::new();
    if !candidate.playable {
        failures.push(ConformanceFailure::NotPlayable);
    }
    if budget.require_same_container && reference.container != candidate.container {
        failures.push(ConformanceFailure::ContainerMismatch);
    }
    if budget.require_same_codecs
        && (reference.video_codec, reference.audio_codec)
            != (candidate.video_codec, candidate.audio_codec)
    {
        failures.push(ConformanceFailure::CodecMismatch);
    }
    if budget.require_same_dimensions
        && (reference.width, reference.height) != (candidate.width, candidate.height)
    {
        failures.push(ConformanceFailure::DimensionMismatch);
    }
    let duration_delta = reference.duration_ms.abs_diff(candidate.duration_ms);
    if duration_delta > budget.duration_tolerance_ms {
        failures.push(ConformanceFailure::DurationDelta {
            actual_ms: duration_delta,
            allowed_ms: budget.duration_tolerance_ms,
        });
    }
    let frame_rate_delta = reference
        .frame_rate_milli
        .abs_diff(candidate.frame_rate_milli);
    if frame_rate_delta > budget.frame_rate_tolerance_milli {
        failures.push(ConformanceFailure::FrameRateDelta {
            actual_milli: frame_rate_delta,
            allowed_milli: budget.frame_rate_tolerance_milli,
        });
    }
    let av_offset = u64::try_from(
        (i128::from(candidate.av_offset_ms) - i128::from(reference.av_offset_ms)).unsigned_abs(),
    )
    .map_err(|_| ConformanceError::InvalidMeasurement)?;
    if av_offset > budget.av_offset_tolerance_ms {
        failures.push(ConformanceFailure::AvOffset {
            actual_ms: av_offset,
            allowed_ms: budget.av_offset_tolerance_ms,
        });
    }
    if measurements.perceptual_milli < budget.minimum_perceptual_milli {
        failures.push(ConformanceFailure::PerceptualScore {
            actual: measurements.perceptual_milli,
            minimum: budget.minimum_perceptual_milli,
        });
    }
    if measurements.waveform_correlation_milli < budget.minimum_waveform_correlation_milli {
        failures.push(ConformanceFailure::WaveformScore {
            actual: measurements.waveform_correlation_milli,
            minimum: budget.minimum_waveform_correlation_milli,
        });
    }
    if measurements.max_caption_timing_delta_ms > budget.caption_timing_tolerance_ms {
        failures.push(ConformanceFailure::CaptionTiming {
            actual_ms: measurements.max_caption_timing_delta_ms,
            allowed_ms: budget.caption_timing_tolerance_ms,
        });
    }
    Ok(ConformanceReport { failures })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressSample {
    pub elapsed_ms: u64,
    /// `None` represents an explicitly indeterminate executor.
    pub basis_points: Option<u16>,
}

pub fn validate_progress_trace(
    samples: &[ProgressSample],
    require_complete: bool,
) -> Result<(), ConformanceError> {
    if samples.is_empty() {
        return Err(ConformanceError::EmptyTrace);
    }
    let mode = samples[0].basis_points.is_some();
    let mut last_time = 0;
    let mut last_progress = 0;
    for (index, sample) in samples.iter().enumerate() {
        if index > 0 && sample.elapsed_ms < last_time {
            return Err(ConformanceError::NonMonotonicTime);
        }
        if sample.basis_points.is_some() != mode {
            return Err(ConformanceError::MixedProgressMode);
        }
        if let Some(progress) = sample.basis_points {
            if progress > 10_000 || progress < last_progress {
                return Err(ConformanceError::NonMonotonicProgress);
            }
            last_progress = progress;
        }
        last_time = sample.elapsed_ms;
    }
    if require_complete && mode && last_progress != 10_000 {
        return Err(ConformanceError::IncompleteProgress);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSample {
    pub elapsed_ms: u64,
    pub memory_bytes: u64,
    pub handles: u64,
    pub threads: u32,
    pub disk_bytes: u64,
    pub cpu_milli_percent: u32,
    pub gpu_milli_percent: u32,
    pub temperature_milli_celsius: u32,
    pub output_latency_ms: u64,
    pub av_drift_ms: i64,
    pub cost_microunits: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    pub max_memory_bytes: u64,
    pub max_memory_growth_bytes: u64,
    pub max_handles: u64,
    pub max_handle_growth: u64,
    pub max_threads: u32,
    pub max_disk_bytes: u64,
    pub max_cpu_milli_percent: u32,
    pub max_gpu_milli_percent: u32,
    pub max_temperature_milli_celsius: u32,
    pub max_output_latency_ms: u64,
    pub max_av_drift_ms: u64,
    pub max_cost_microunits: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceFailure {
    MemoryLimit,
    MemoryGrowth,
    HandleLimit,
    HandleGrowth,
    ThreadLimit,
    DiskLimit,
    CpuLimit,
    GpuLimit,
    TemperatureLimit,
    LatencyLimit,
    AvDrift,
    CostLimit,
}

pub fn evaluate_resource_trace(
    samples: &[ResourceSample],
    budget: ResourceBudget,
) -> Result<Vec<ResourceFailure>, ConformanceError> {
    if samples.is_empty() {
        return Err(ConformanceError::EmptyTrace);
    }
    if budget.max_memory_bytes == 0
        || budget.max_handles == 0
        || budget.max_threads == 0
        || budget.max_disk_bytes == 0
        || budget.max_cpu_milli_percent == 0
        || budget.max_gpu_milli_percent == 0
        || budget.max_temperature_milli_celsius == 0
        || budget.max_output_latency_ms == 0
    {
        return Err(ConformanceError::InvalidBudget);
    }
    for pair in samples.windows(2) {
        if pair[1].elapsed_ms < pair[0].elapsed_ms
            || pair[1].cost_microunits < pair[0].cost_microunits
        {
            return Err(ConformanceError::NonMonotonicTime);
        }
    }

    let first = samples.first().ok_or(ConformanceError::EmptyTrace)?;
    let last = samples.last().ok_or(ConformanceError::EmptyTrace)?;
    let mut failures = Vec::new();
    if samples
        .iter()
        .any(|sample| sample.memory_bytes > budget.max_memory_bytes)
    {
        failures.push(ResourceFailure::MemoryLimit);
    }
    if last.memory_bytes.saturating_sub(first.memory_bytes) > budget.max_memory_growth_bytes {
        failures.push(ResourceFailure::MemoryGrowth);
    }
    if samples
        .iter()
        .any(|sample| sample.handles > budget.max_handles)
    {
        failures.push(ResourceFailure::HandleLimit);
    }
    if last.handles.saturating_sub(first.handles) > budget.max_handle_growth {
        failures.push(ResourceFailure::HandleGrowth);
    }
    if samples
        .iter()
        .any(|sample| sample.threads > budget.max_threads)
    {
        failures.push(ResourceFailure::ThreadLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.disk_bytes > budget.max_disk_bytes)
    {
        failures.push(ResourceFailure::DiskLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.cpu_milli_percent > budget.max_cpu_milli_percent)
    {
        failures.push(ResourceFailure::CpuLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.gpu_milli_percent > budget.max_gpu_milli_percent)
    {
        failures.push(ResourceFailure::GpuLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.temperature_milli_celsius > budget.max_temperature_milli_celsius)
    {
        failures.push(ResourceFailure::TemperatureLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.output_latency_ms > budget.max_output_latency_ms)
    {
        failures.push(ResourceFailure::LatencyLimit);
    }
    if samples
        .iter()
        .any(|sample| sample.av_drift_ms.unsigned_abs() > budget.max_av_drift_ms)
    {
        failures.push(ResourceFailure::AvDrift);
    }
    if last.cost_microunits > budget.max_cost_microunits {
        failures.push(ResourceFailure::CostLimit);
    }
    failures.sort_unstable();
    failures.dedup();
    Ok(failures)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyBudget {
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub maximum_ms: u64,
}

impl LatencyBudget {
    pub fn validate(self) -> Result<Self, ConformanceError> {
        if self.p50_ms == 0
            || self.p50_ms > self.p95_ms
            || self.p95_ms > self.p99_ms
            || self.p99_ms > self.maximum_ms
        {
            return Err(ConformanceError::InvalidBudget);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LatencyFailure {
    P50,
    P95,
    P99,
    Maximum,
}

pub fn evaluate_latency_distribution(
    samples_ms: &[u64],
    budget: LatencyBudget,
) -> Result<Vec<LatencyFailure>, ConformanceError> {
    if samples_ms.is_empty() {
        return Err(ConformanceError::EmptyTrace);
    }
    let budget = budget.validate()?;
    let mut ordered = samples_ms.to_vec();
    ordered.sort_unstable();
    let mut failures = Vec::new();
    if nearest_rank(&ordered, 5_000)? > budget.p50_ms {
        failures.push(LatencyFailure::P50);
    }
    if nearest_rank(&ordered, 9_500)? > budget.p95_ms {
        failures.push(LatencyFailure::P95);
    }
    if nearest_rank(&ordered, 9_900)? > budget.p99_ms {
        failures.push(LatencyFailure::P99);
    }
    if ordered
        .last()
        .copied()
        .ok_or(ConformanceError::EmptyTrace)?
        > budget.maximum_ms
    {
        failures.push(LatencyFailure::Maximum);
    }
    Ok(failures)
}

fn nearest_rank(ordered: &[u64], basis_points: u16) -> Result<u64, ConformanceError> {
    if ordered.is_empty() || basis_points == 0 || basis_points > 10_000 {
        return Err(ConformanceError::InvalidMeasurement);
    }
    let numerator = ordered
        .len()
        .checked_mul(usize::from(basis_points))
        .ok_or(ConformanceError::InvalidMeasurement)?;
    let rank = numerator.div_ceil(10_000).max(1);
    ordered
        .get(rank - 1)
        .copied()
        .ok_or(ConformanceError::InvalidMeasurement)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultScenario {
    DeviceLost,
    PermissionDenied,
    ProcessCrash,
    DiskFull,
    NetworkLost,
    ProviderError,
    ProviderOutage,
    ProviderQuota,
    ManagedOutputDrift,
    Cancellation,
    UnsupportedCodec,
    HardwareEncoderFailure,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDisposition {
    Resume,
    RecoverArtifact,
    FailActionable,
    Fallback,
    SuppressPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultExpectation {
    pub scenario: FaultScenario,
    pub disposition: RecoveryDisposition,
}

pub fn verify_fault_matrix(
    expected: &[FaultExpectation],
    actual: &[FaultExpectation],
) -> Result<(), ConformanceError> {
    if expected.is_empty() {
        return Err(ConformanceError::EmptyFaultMatrix);
    }
    if expected.len() != actual.len() {
        return Err(ConformanceError::FaultMatrixCardinality);
    }
    if expected.iter().enumerate().any(|(index, item)| {
        expected[index + 1..]
            .iter()
            .any(|other| other.scenario == item.scenario)
    }) || actual.iter().enumerate().any(|(index, item)| {
        actual[index + 1..]
            .iter()
            .any(|other| other.scenario == item.scenario)
    }) {
        return Err(ConformanceError::FaultMatrixCardinality);
    }
    for expectation in expected {
        let matches = actual
            .iter()
            .filter(|result| result.scenario == expectation.scenario);
        let results: Vec<_> = matches.collect();
        if results.len() != 1 || results[0].disposition != expectation.disposition {
            return Err(ConformanceError::FaultMismatch(expectation.scenario));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingResult {
    Managed,
    Native,
    ExternalProvider,
    RejectedBeforeInvocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingObservation {
    pub result: RoutingResult,
    pub managed_invocations: u16,
    pub native_invocations: u16,
    pub external_invocations: u16,
}

pub fn verify_routing_observation(
    expected: RoutingResult,
    actual: RoutingObservation,
) -> Result<(), ConformanceError> {
    if expected != actual.result {
        return Err(ConformanceError::RoutingMismatch);
    }
    let valid_invocations = match expected {
        RoutingResult::Managed => {
            actual.managed_invocations == 1
                && actual.native_invocations == 0
                && actual.external_invocations == 0
        }
        RoutingResult::Native => {
            actual.managed_invocations == 0
                && actual.native_invocations == 1
                && actual.external_invocations == 0
        }
        RoutingResult::ExternalProvider => {
            actual.managed_invocations == 0
                && actual.native_invocations == 0
                && actual.external_invocations == 1
        }
        RoutingResult::RejectedBeforeInvocation => {
            actual.managed_invocations == 0
                && actual.native_invocations == 0
                && actual.external_invocations == 0
        }
    };
    if !valid_invocations {
        return Err(ConformanceError::UnexpectedInvocation);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalResultObservation<'a> {
    pub logical_key: &'a str,
    pub checksum_sha256: &'a str,
    /// Number of successful publication mutations performed by this attempt.
    pub publication_effects: u16,
    /// Number of billable provider effects performed by this attempt.
    pub billable_effects: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalResultSummary {
    pub attempts: u16,
    pub publication_effects: u16,
    pub billable_effects: u16,
}

pub fn verify_single_logical_result(
    observations: &[LogicalResultObservation<'_>],
    require_publication: bool,
) -> Result<LogicalResultSummary, ConformanceError> {
    let first = observations.first().ok_or(ConformanceError::EmptyTrace)?;
    if !safe_object_key(first.logical_key) || !lower_hex(first.checksum_sha256, 64) {
        return Err(ConformanceError::InvalidLogicalResult);
    }
    let mut publication_effects = 0_u16;
    let mut billable_effects = 0_u16;
    for observation in observations {
        if observation.logical_key != first.logical_key
            || observation.checksum_sha256 != first.checksum_sha256
            || observation.publication_effects > 1
            || observation.billable_effects > 1
        {
            return Err(ConformanceError::InvalidLogicalResult);
        }
        publication_effects = publication_effects
            .checked_add(observation.publication_effects)
            .ok_or(ConformanceError::InvalidLogicalResult)?;
        billable_effects = billable_effects
            .checked_add(observation.billable_effects)
            .ok_or(ConformanceError::InvalidLogicalResult)?;
    }
    if publication_effects > 1
        || billable_effects > 1
        || (require_publication && publication_effects != 1)
    {
        return Err(ConformanceError::DuplicateLogicalEffect);
    }
    Ok(LogicalResultSummary {
        attempts: u16::try_from(observations.len())
            .map_err(|_| ConformanceError::InvalidLogicalResult)?,
        publication_effects,
        billable_effects,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegressionSeed {
    Quality,
    Sync,
    Recovery,
    Resource,
    Routing,
    ProviderLimit,
    ManagedOutputDrift,
    Fallback,
    RepeatCost,
}

impl RegressionSeed {
    pub const ALL: [Self; 9] = [
        Self::Quality,
        Self::Sync,
        Self::Recovery,
        Self::Resource,
        Self::Routing,
        Self::ProviderLimit,
        Self::ManagedOutputDrift,
        Self::Fallback,
        Self::RepeatCost,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegressionGate {
    Perceptual,
    AvSync,
    RecoveryState,
    ResourceTrend,
    ExecutorRouting,
    ProviderPreflight,
    OutputCompatibility,
    FallbackPolicy,
    CostIdempotency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegressionObservation {
    pub seed: RegressionSeed,
    pub failed_gate: RegressionGate,
}

pub fn verify_seeded_regressions(
    observations: &[RegressionObservation],
) -> Result<(), ConformanceError> {
    const EXPECTED: [RegressionObservation; 9] = [
        RegressionObservation {
            seed: RegressionSeed::Quality,
            failed_gate: RegressionGate::Perceptual,
        },
        RegressionObservation {
            seed: RegressionSeed::Sync,
            failed_gate: RegressionGate::AvSync,
        },
        RegressionObservation {
            seed: RegressionSeed::Recovery,
            failed_gate: RegressionGate::RecoveryState,
        },
        RegressionObservation {
            seed: RegressionSeed::Resource,
            failed_gate: RegressionGate::ResourceTrend,
        },
        RegressionObservation {
            seed: RegressionSeed::Routing,
            failed_gate: RegressionGate::ExecutorRouting,
        },
        RegressionObservation {
            seed: RegressionSeed::ProviderLimit,
            failed_gate: RegressionGate::ProviderPreflight,
        },
        RegressionObservation {
            seed: RegressionSeed::ManagedOutputDrift,
            failed_gate: RegressionGate::OutputCompatibility,
        },
        RegressionObservation {
            seed: RegressionSeed::Fallback,
            failed_gate: RegressionGate::FallbackPolicy,
        },
        RegressionObservation {
            seed: RegressionSeed::RepeatCost,
            failed_gate: RegressionGate::CostIdempotency,
        },
    ];
    if observations.len() != EXPECTED.len()
        || EXPECTED.iter().any(|expected| {
            observations
                .iter()
                .filter(|actual| actual.seed == expected.seed && *actual == expected)
                .count()
                != 1
        })
    {
        return Err(ConformanceError::RegressionGateMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuzzEnvelopeKind {
    Label,
    ObjectKey,
    Sha256,
    Progress,
}

/// Parses the tiny untrusted envelope used by the deterministic conformance
/// fuzz corpus. The parser is deliberately independent from JSON and media
/// decoding so arbitrary bytes can exercise boundary validation without file,
/// provider, or hardware access.
pub fn validate_conformance_fuzz_envelope(
    input: &[u8],
) -> Result<FuzzEnvelopeKind, ConformanceError> {
    if input.is_empty() || input.len() > 512 || input.contains(&0) {
        return Err(ConformanceError::InvalidFuzzEnvelope);
    }
    let value = std::str::from_utf8(input).map_err(|_| ConformanceError::InvalidFuzzEnvelope)?;
    let (kind, payload) = value
        .split_once(':')
        .ok_or(ConformanceError::InvalidFuzzEnvelope)?;
    if payload.is_empty() || payload.contains(':') {
        return Err(ConformanceError::InvalidFuzzEnvelope);
    }
    match kind {
        "label" if safe_label(payload) => Ok(FuzzEnvelopeKind::Label),
        "key" if safe_object_key(payload) => Ok(FuzzEnvelopeKind::ObjectKey),
        "sha256" if lower_hex(payload, 64) => Ok(FuzzEnvelopeKind::Sha256),
        "progress"
            if payload.bytes().all(|byte| byte.is_ascii_digit())
                && payload
                    .parse::<u16>()
                    .is_ok_and(|basis_points| basis_points <= 10_000) =>
        {
            Ok(FuzzEnvelopeKind::Progress)
        }
        _ => Err(ConformanceError::InvalidFuzzEnvelope),
    }
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.+ ()".contains(character))
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn safe_object_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1_024
        && !value.contains("://")
        && !value.contains(['?', '#', '\\'])
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceError {
    #[error("artifact provenance is invalid or unsafe")]
    InvalidProvenance,
    #[error("conformance measurement is outside its normalized range")]
    InvalidMeasurement,
    #[error("conformance budget is invalid")]
    InvalidBudget,
    #[error("conformance trace is empty")]
    EmptyTrace,
    #[error("trace time or accumulated cost is non-monotonic")]
    NonMonotonicTime,
    #[error("trace mixes determinate and indeterminate progress")]
    MixedProgressMode,
    #[error("progress is non-monotonic or out of range")]
    NonMonotonicProgress,
    #[error("completed determinate trace does not end at 10000 basis points")]
    IncompleteProgress,
    #[error("fault matrix is empty")]
    EmptyFaultMatrix,
    #[error("fault matrix has missing, duplicate, or unexpected scenarios")]
    FaultMatrixCardinality,
    #[error("fault result does not match expectation for {0:?}")]
    FaultMismatch(FaultScenario),
    #[error("executor route differs from the declared matrix")]
    RoutingMismatch,
    #[error("executor invocation count violates the declared route")]
    UnexpectedInvocation,
    #[error("logical result identity is invalid or inconsistent")]
    InvalidLogicalResult,
    #[error("logical result repeated a publication or billable effect")]
    DuplicateLogicalEffect,
    #[error("seeded regression did not fail its one declared gate")]
    RegressionGateMismatch,
    #[error("conformance fuzz envelope is invalid")]
    InvalidFuzzEnvelope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> MediaProbeSummary {
        MediaProbeSummary {
            container: ContainerFormat::Mp4,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            width: 1_920,
            height: 1_080,
            duration_ms: 10_000,
            frame_rate_milli: 30_000,
            av_offset_ms: 5,
            playable: true,
        }
    }

    fn budget() -> ConformanceBudget {
        ConformanceBudget {
            duration_tolerance_ms: 100,
            frame_rate_tolerance_milli: 100,
            av_offset_tolerance_ms: 20,
            minimum_perceptual_milli: 900,
            minimum_waveform_correlation_milli: 950,
            caption_timing_tolerance_ms: 200,
            require_same_dimensions: true,
            require_same_container: false,
            require_same_codecs: true,
        }
    }

    #[test]
    fn provenance_fails_closed_across_offline_hardware_and_provider_lanes() {
        let offline = ArtifactProvenance {
            schema_version: CONFORMANCE_SCHEMA_VERSION,
            fixture_id: "synthetic-bars-v1".into(),
            fixture_sha256: "a".repeat(64),
            source_revision: "b".repeat(40),
            tool_version: "frame-media-0.1.0".into(),
            runtime_version: "gstreamer-1.28.0".into(),
            os_family: "linux".into(),
            architecture: "x86_64".into(),
            hardware_class: None,
            provider_revision: None,
            profile_version: 1,
            executor: MediaExecutorKind::NativeGstreamer,
            lane: EvidenceLane::OfflineSynthetic,
        };
        offline.clone().validate().expect("offline provenance");

        let mut forged_remote = offline;
        forged_remote.lane = EvidenceLane::RemoteManaged;
        forged_remote.executor = MediaExecutorKind::CloudflareMedia;
        assert_eq!(
            forged_remote.validate(),
            Err(ConformanceError::InvalidProvenance)
        );

        let external = ArtifactProvenance {
            schema_version: CONFORMANCE_SCHEMA_VERSION,
            fixture_id: "synthetic-captions-v1".into(),
            fixture_sha256: "c".repeat(64),
            source_revision: "d".repeat(40),
            tool_version: "frame-media-0.1.0".into(),
            runtime_version: "provider-adapter-v1".into(),
            os_family: "provider".into(),
            architecture: "remote".into(),
            hardware_class: None,
            provider_revision: Some("sandbox-model-v1".into()),
            profile_version: 1,
            executor: MediaExecutorKind::ExternalProvider,
            lane: EvidenceLane::ExternalProvider,
        };
        external.validate().expect("external provider provenance");
    }

    #[test]
    fn cross_executor_comparison_does_not_require_byte_identity() {
        let candidate = MediaProbeSummary {
            container: ContainerFormat::QuickTime,
            duration_ms: 10_050,
            av_offset_ms: -10,
            ..probe()
        };
        let report = compare_media(
            probe(),
            candidate,
            PairMeasurements {
                perceptual_milli: 950,
                waveform_correlation_milli: 980,
                max_caption_timing_delta_ms: 100,
            },
            budget(),
        )
        .expect("report");
        assert!(report.passed());
    }

    #[test]
    fn seeded_quality_and_sync_regressions_hit_specific_gates() {
        let candidate = MediaProbeSummary {
            av_offset_ms: 50,
            ..probe()
        };
        let report = compare_media(
            probe(),
            candidate,
            PairMeasurements {
                perceptual_milli: 800,
                waveform_correlation_milli: 980,
                max_caption_timing_delta_ms: 100,
            },
            budget(),
        )
        .expect("report");
        assert!(report.failures.contains(&ConformanceFailure::AvOffset {
            actual_ms: 45,
            allowed_ms: 20
        }));
        assert!(
            report
                .failures
                .contains(&ConformanceFailure::PerceptualScore {
                    actual: 800,
                    minimum: 900
                })
        );
    }

    #[test]
    fn determinate_progress_must_be_monotonic_and_complete() {
        let trace = [
            ProgressSample {
                elapsed_ms: 0,
                basis_points: Some(0),
            },
            ProgressSample {
                elapsed_ms: 10,
                basis_points: Some(5_000),
            },
            ProgressSample {
                elapsed_ms: 20,
                basis_points: Some(10_000),
            },
        ];
        validate_progress_trace(&trace, true).expect("trace");
        let regressed = [trace[1], trace[0]];
        assert_eq!(
            validate_progress_trace(&regressed, false),
            Err(ConformanceError::NonMonotonicTime)
        );
    }

    #[test]
    fn resource_trace_detects_leak_trends() {
        let samples = [
            ResourceSample {
                elapsed_ms: 0,
                memory_bytes: 100,
                handles: 10,
                threads: 2,
                disk_bytes: 0,
                cpu_milli_percent: 1_000,
                gpu_milli_percent: 500,
                temperature_milli_celsius: 40_000,
                output_latency_ms: 20,
                av_drift_ms: 0,
                cost_microunits: 0,
            },
            ResourceSample {
                elapsed_ms: 10,
                memory_bytes: 160,
                handles: 15,
                threads: 2,
                disk_bytes: 10,
                cpu_milli_percent: 1_000,
                gpu_milli_percent: 500,
                temperature_milli_celsius: 40_000,
                output_latency_ms: 20,
                av_drift_ms: 1,
                cost_microunits: 1,
            },
        ];
        let failures = evaluate_resource_trace(
            &samples,
            ResourceBudget {
                max_memory_bytes: 1_000,
                max_memory_growth_bytes: 50,
                max_handles: 100,
                max_handle_growth: 4,
                max_threads: 10,
                max_disk_bytes: 1_000,
                max_cpu_milli_percent: 10_000,
                max_gpu_milli_percent: 10_000,
                max_temperature_milli_celsius: 90_000,
                max_output_latency_ms: 100,
                max_av_drift_ms: 10,
                max_cost_microunits: 10,
            },
        )
        .expect("trace");
        assert_eq!(
            failures,
            vec![ResourceFailure::MemoryGrowth, ResourceFailure::HandleGrowth]
        );
    }

    #[test]
    fn latency_distribution_uses_deterministic_nearest_rank() {
        let samples: Vec<_> = (1..=100).collect();
        let failures = evaluate_latency_distribution(
            &samples,
            LatencyBudget {
                p50_ms: 50,
                p95_ms: 94,
                p99_ms: 99,
                maximum_ms: 100,
            },
        )
        .expect("latency report");
        assert_eq!(failures, vec![LatencyFailure::P95]);
    }

    #[test]
    fn fault_matrix_rejects_missing_or_duplicate_scenarios() {
        let expected = [
            FaultExpectation {
                scenario: FaultScenario::NetworkLost,
                disposition: RecoveryDisposition::Resume,
            },
            FaultExpectation {
                scenario: FaultScenario::ProviderError,
                disposition: RecoveryDisposition::Fallback,
            },
        ];
        verify_fault_matrix(&expected, &expected).expect("matrix");
        assert_eq!(
            verify_fault_matrix(&expected, &expected[..1]),
            Err(ConformanceError::FaultMatrixCardinality)
        );
        assert_eq!(
            verify_fault_matrix(&[expected[0], expected[0]], &[expected[0], expected[0]]),
            Err(ConformanceError::FaultMatrixCardinality)
        );
    }

    #[test]
    fn preflight_rejection_proves_zero_executor_invocations() {
        verify_routing_observation(
            RoutingResult::RejectedBeforeInvocation,
            RoutingObservation {
                result: RoutingResult::RejectedBeforeInvocation,
                managed_invocations: 0,
                native_invocations: 0,
                external_invocations: 0,
            },
        )
        .expect("preflight rejection");
        assert_eq!(
            verify_routing_observation(
                RoutingResult::RejectedBeforeInvocation,
                RoutingObservation {
                    result: RoutingResult::RejectedBeforeInvocation,
                    managed_invocations: 1,
                    native_invocations: 0,
                    external_invocations: 0,
                },
            ),
            Err(ConformanceError::UnexpectedInvocation)
        );
    }

    #[test]
    fn replays_preserve_one_key_one_publication_and_one_billable_effect() {
        let checksum = "c".repeat(64);
        let observations = [
            LogicalResultObservation {
                logical_key: "tenants/t/videos/v/derivatives/p/v1/result.mp4",
                checksum_sha256: &checksum,
                publication_effects: 1,
                billable_effects: 1,
            },
            LogicalResultObservation {
                logical_key: "tenants/t/videos/v/derivatives/p/v1/result.mp4",
                checksum_sha256: &checksum,
                publication_effects: 0,
                billable_effects: 0,
            },
        ];
        assert_eq!(
            verify_single_logical_result(&observations, true).expect("idempotent replay"),
            LogicalResultSummary {
                attempts: 2,
                publication_effects: 1,
                billable_effects: 1
            }
        );
        let repeated = [observations[0], observations[0]];
        assert_eq!(
            verify_single_logical_result(&repeated, true),
            Err(ConformanceError::DuplicateLogicalEffect)
        );
    }

    #[test]
    fn every_required_seed_maps_to_exactly_one_regression_gate() {
        let observations = [
            RegressionObservation {
                seed: RegressionSeed::Quality,
                failed_gate: RegressionGate::Perceptual,
            },
            RegressionObservation {
                seed: RegressionSeed::Sync,
                failed_gate: RegressionGate::AvSync,
            },
            RegressionObservation {
                seed: RegressionSeed::Recovery,
                failed_gate: RegressionGate::RecoveryState,
            },
            RegressionObservation {
                seed: RegressionSeed::Resource,
                failed_gate: RegressionGate::ResourceTrend,
            },
            RegressionObservation {
                seed: RegressionSeed::Routing,
                failed_gate: RegressionGate::ExecutorRouting,
            },
            RegressionObservation {
                seed: RegressionSeed::ProviderLimit,
                failed_gate: RegressionGate::ProviderPreflight,
            },
            RegressionObservation {
                seed: RegressionSeed::ManagedOutputDrift,
                failed_gate: RegressionGate::OutputCompatibility,
            },
            RegressionObservation {
                seed: RegressionSeed::Fallback,
                failed_gate: RegressionGate::FallbackPolicy,
            },
            RegressionObservation {
                seed: RegressionSeed::RepeatCost,
                failed_gate: RegressionGate::CostIdempotency,
            },
        ];
        verify_seeded_regressions(&observations).expect("regression gates");
        let mut wrong = observations;
        wrong[0].failed_gate = RegressionGate::AvSync;
        assert_eq!(
            verify_seeded_regressions(&wrong),
            Err(ConformanceError::RegressionGateMismatch)
        );
    }

    #[test]
    fn fuzz_envelope_is_bounded_and_fail_closed() {
        assert_eq!(
            validate_conformance_fuzz_envelope(b"progress:10000"),
            Ok(FuzzEnvelopeKind::Progress)
        );
        assert_eq!(
            validate_conformance_fuzz_envelope(b"progress:10001"),
            Err(ConformanceError::InvalidFuzzEnvelope)
        );
        assert_eq!(
            validate_conformance_fuzz_envelope(b"key:../private"),
            Err(ConformanceError::InvalidFuzzEnvelope)
        );
        assert_eq!(
            validate_conformance_fuzz_envelope(&vec![b'a'; 513]),
            Err(ConformanceError::InvalidFuzzEnvelope)
        );
    }
}
