use thiserror::Error;

use crate::{AudioCodec, ContainerFormat, MediaExecutorKind, VideoCodec};

pub const CONFORMANCE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactProvenance {
    pub schema_version: u16,
    pub fixture_id: String,
    pub tool_version: String,
    pub runtime_version: String,
    pub profile_version: u16,
    pub executor: MediaExecutorKind,
}

impl ArtifactProvenance {
    pub fn validate(self) -> Result<Self, ConformanceError> {
        if self.schema_version != CONFORMANCE_SCHEMA_VERSION
            || !safe_label(&self.fixture_id)
            || !safe_label(&self.tool_version)
            || !safe_label(&self.runtime_version)
            || self.profile_version == 0
        {
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
pub enum FaultScenario {
    DeviceLost,
    ProcessCrash,
    DiskFull,
    NetworkLost,
    ProviderError,
    Cancellation,
    UnsupportedCodec,
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

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.+ ()".contains(character))
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
    }
}
