//! Bounded execution of the canonical Studio edit plan.
//!
//! [`StudioTimelineCompiler`](crate::StudioTimelineCompiler) remains the sole
//! edit interpreter. This module turns its immutable output into the exact
//! windows consumed by native preview and export adapters. In particular, it
//! splits compiled spans at optional-track coverage gaps so neither adapter has
//! to reinterpret timeline, speed, gain, mute, layout, or gap semantics.

use std::cmp::Ordering;

use thiserror::Error;

use crate::{
    AudioStyle, CancellationToken, CanonicalEditPlan, CompositeStyle, ExactDuration,
    GapDisposition, MAX_STUDIO_EDITS, MAX_STUDIO_GAP_INSTRUCTIONS, RationalTime, Sha256Digest,
    StudioError, TrackKind,
};

/// A compiler plan has at most one window for every span plus two new
/// boundaries for every optional-track gap.
pub const MAX_STUDIO_EDIT_EXECUTION_WINDOWS: usize = MAX_STUDIO_EDITS
    .saturating_mul(2)
    .saturating_add(1)
    .saturating_add(MAX_STUDIO_GAP_INSTRUCTIONS.saturating_mul(2));

/// Prevent one cooperative export poll from monopolizing its caller.
pub const MAX_STUDIO_EDIT_EXECUTION_BATCH: usize = 256;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StudioEditExecutionError {
    #[error("canonical Studio edit plan is invalid: {0}")]
    InvalidPlan(#[from] StudioError),
    #[error("Studio edit execution window limit is invalid")]
    InvalidWindowLimit,
    #[error("Studio edit execution exceeds its declared window limit")]
    WindowLimitExceeded,
    #[error("Studio edit execution batch limit is invalid")]
    InvalidBatchLimit,
    #[error("Studio edit execution was cancelled")]
    Cancelled,
    #[error("Studio edit execution point is outside the output timeline")]
    OutputOutsideTimeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioAudioExecution {
    /// False means the canonical plan requires synthesized silence because the
    /// original has no coverage in this window.
    pub source_available: bool,
    pub style: AudioStyle,
}

impl StudioAudioExecution {
    #[must_use]
    pub const fn silent(self) -> bool {
        !self.source_available || self.style.muted
    }
}

/// One maximal interval over which timeline rate, composition state, and
/// optional-track availability are constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioEditExecutionWindow {
    pub source_start: RationalTime,
    pub source_end: RationalTime,
    pub output_start: ExactDuration,
    pub output_end: ExactDuration,
    pub speed_numerator: u32,
    pub speed_denominator: u32,
    pub style: CompositeStyle,
    pub camera_source_available: bool,
    pub microphone: StudioAudioExecution,
    pub system_audio: StudioAudioExecution,
}

impl StudioEditExecutionWindow {
    #[must_use]
    pub const fn playback_rate(self) -> (u32, u32) {
        (self.speed_numerator, self.speed_denominator)
    }

    pub fn contains_output(self, output: ExactDuration) -> Result<bool, StudioEditExecutionError> {
        Ok(
            compare_duration(output, self.output_start)? != Ordering::Less
                && compare_duration(output, self.output_end)? == Ordering::Less,
        )
    }

    pub fn source_time_at_output(
        self,
        output: ExactDuration,
    ) -> Result<ExactDuration, StudioEditExecutionError> {
        if !self.contains_output(output)? {
            return Err(StudioEditExecutionError::OutputOutsideTimeline);
        }
        let source_start = ExactDuration::new(
            u128::from(self.source_start.ticks()),
            u128::from(self.source_start.time_base().ticks_per_second()),
        )?;
        let output_offset = duration_sub(output, self.output_start)?;
        source_start
            .checked_add(output_offset.scaled(self.speed_numerator, self.speed_denominator)?)
            .map_err(Into::into)
    }
}

/// Immutable execution windows plus a bounded cooperative export cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioEditExecutor {
    plan_digest: Sha256Digest,
    output_duration: ExactDuration,
    windows: Vec<StudioEditExecutionWindow>,
    cursor: usize,
}

impl StudioEditExecutor {
    pub fn compile(
        plan: &CanonicalEditPlan,
        maximum_windows: usize,
        cancellation: &CancellationToken,
    ) -> Result<Self, StudioEditExecutionError> {
        if maximum_windows == 0 || maximum_windows > MAX_STUDIO_EDIT_EXECUTION_WINDOWS {
            return Err(StudioEditExecutionError::InvalidWindowLimit);
        }
        if cancellation.is_cancelled() {
            return Err(StudioEditExecutionError::Cancelled);
        }
        plan.validate()?;

        let mut events_by_span = vec![Vec::new(); plan.spans.len()];
        for gap in &plan.gaps {
            if cancellation.is_cancelled() {
                return Err(StudioEditExecutionError::Cancelled);
            }
            let span_index = containing_span(plan, gap.source_start, gap.source_end)?;
            let track = match (gap.track, gap.disposition) {
                (TrackKind::Camera, GapDisposition::HideCamera) => GapTrack::Camera,
                (TrackKind::Microphone, GapDisposition::InsertSilence) => GapTrack::Microphone,
                (TrackKind::SystemAudio, GapDisposition::InsertSilence) => GapTrack::SystemAudio,
                (TrackKind::Screen, _)
                | (TrackKind::Camera, GapDisposition::InsertSilence)
                | (TrackKind::Microphone | TrackKind::SystemAudio, GapDisposition::HideCamera) => {
                    return Err(StudioError::InvalidCompiledPlan.into());
                }
            };
            events_by_span[span_index].push(GapEvent {
                at: gap.source_start,
                track,
                opening: true,
            });
            events_by_span[span_index].push(GapEvent {
                at: gap.source_end,
                track,
                opening: false,
            });
        }

        let mut windows = Vec::with_capacity(
            maximum_windows.min(
                plan.spans
                    .len()
                    .saturating_add(plan.gaps.len().saturating_mul(2)),
            ),
        );
        for (span_index, span) in plan.spans.iter().enumerate() {
            if cancellation.is_cancelled() {
                return Err(StudioEditExecutionError::Cancelled);
            }
            let events = &mut events_by_span[span_index];
            events.sort_by(|left, right| {
                compare_time_infallible(left.at, right.at)
                    // Closing before opening makes adjacent gaps half-open and
                    // avoids a transient false overlap at their shared point.
                    .then_with(|| left.opening.cmp(&right.opening))
                    .then_with(|| left.track.cmp(&right.track))
            });

            let mut active = ActiveGaps::default();
            let mut source_cursor = span.source_start;
            let mut event_index = 0_usize;
            while event_index < events.len() {
                let boundary = events[event_index].at;
                if compare_time_infallible(source_cursor, boundary) == Ordering::Less {
                    push_window(
                        &mut windows,
                        maximum_windows,
                        span,
                        source_cursor,
                        boundary,
                        active,
                    )?;
                    source_cursor = boundary;
                }
                while event_index < events.len()
                    && compare_time_infallible(events[event_index].at, boundary) == Ordering::Equal
                {
                    active.apply(events[event_index])?;
                    event_index = event_index.saturating_add(1);
                }
            }
            if active != ActiveGaps::default() {
                return Err(StudioError::InvalidCompiledPlan.into());
            }
            if compare_time_infallible(source_cursor, span.source_end) == Ordering::Less {
                push_window(
                    &mut windows,
                    maximum_windows,
                    span,
                    source_cursor,
                    span.source_end,
                    active,
                )?;
            }
        }
        if windows.is_empty() {
            return Err(StudioError::InvalidCompiledPlan.into());
        }

        Ok(Self {
            plan_digest: plan.digest(),
            output_duration: plan.output_duration,
            windows,
            cursor: 0,
        })
    }

    #[must_use]
    pub const fn plan_digest(&self) -> Sha256Digest {
        self.plan_digest
    }

    #[must_use]
    pub const fn output_duration(&self) -> ExactDuration {
        self.output_duration
    }

    #[must_use]
    pub fn windows(&self) -> &[StudioEditExecutionWindow] {
        &self.windows
    }

    #[must_use]
    pub fn remaining_windows(&self) -> usize {
        self.windows.len().saturating_sub(self.cursor)
    }

    /// Returns a borrowed batch and advances only after cancellation and batch
    /// bounds have been checked.
    pub fn next_batch(
        &mut self,
        maximum_windows: usize,
        cancellation: &CancellationToken,
    ) -> Result<&[StudioEditExecutionWindow], StudioEditExecutionError> {
        if maximum_windows == 0 || maximum_windows > MAX_STUDIO_EDIT_EXECUTION_BATCH {
            return Err(StudioEditExecutionError::InvalidBatchLimit);
        }
        if cancellation.is_cancelled() {
            return Err(StudioEditExecutionError::Cancelled);
        }
        let end = self
            .cursor
            .saturating_add(maximum_windows)
            .min(self.windows.len());
        let batch = &self.windows[self.cursor..end];
        self.cursor = end;
        Ok(batch)
    }

    pub fn rewind(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), StudioEditExecutionError> {
        if cancellation.is_cancelled() {
            return Err(StudioEditExecutionError::Cancelled);
        }
        self.cursor = 0;
        Ok(())
    }

    pub fn window_at_output(
        &self,
        output: ExactDuration,
        cancellation: &CancellationToken,
    ) -> Result<StudioEditExecutionWindow, StudioEditExecutionError> {
        if cancellation.is_cancelled() {
            return Err(StudioEditExecutionError::Cancelled);
        }
        if compare_duration(output, self.output_duration)? != Ordering::Less {
            return Err(StudioEditExecutionError::OutputOutsideTimeline);
        }
        let mut low = 0_usize;
        let mut high = self.windows.len();
        while low < high {
            let middle = low + (high - low) / 2;
            let window = self.windows[middle];
            if compare_duration(output, window.output_start)? == Ordering::Less {
                high = middle;
            } else if compare_duration(output, window.output_end)? != Ordering::Less {
                low = middle.saturating_add(1);
            } else {
                return Ok(window);
            }
        }
        Err(StudioEditExecutionError::OutputOutsideTimeline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GapTrack {
    Camera,
    Microphone,
    SystemAudio,
}

#[derive(Debug, Clone, Copy)]
struct GapEvent {
    at: RationalTime,
    track: GapTrack,
    opening: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ActiveGaps {
    camera: u16,
    microphone: u16,
    system_audio: u16,
}

impl ActiveGaps {
    fn apply(&mut self, event: GapEvent) -> Result<(), StudioEditExecutionError> {
        let counter = match event.track {
            GapTrack::Camera => &mut self.camera,
            GapTrack::Microphone => &mut self.microphone,
            GapTrack::SystemAudio => &mut self.system_audio,
        };
        if event.opening {
            *counter = counter
                .checked_add(1)
                .ok_or(StudioError::InvalidCompiledPlan)?;
        } else {
            *counter = counter
                .checked_sub(1)
                .ok_or(StudioError::InvalidCompiledPlan)?;
        }
        Ok(())
    }
}

fn containing_span(
    plan: &CanonicalEditPlan,
    start: RationalTime,
    end: RationalTime,
) -> Result<usize, StudioEditExecutionError> {
    let mut low = 0_usize;
    let mut high = plan.spans.len();
    while low < high {
        let middle = low + (high - low) / 2;
        let span = &plan.spans[middle];
        if compare_time_infallible(start, span.source_start) == Ordering::Less {
            high = middle;
        } else if compare_time_infallible(start, span.source_end) != Ordering::Less {
            low = middle.saturating_add(1);
        } else if compare_time_infallible(end, span.source_end) != Ordering::Greater {
            return Ok(middle);
        } else {
            return Err(StudioError::InvalidCompiledPlan.into());
        }
    }
    Err(StudioError::InvalidCompiledPlan.into())
}

fn push_window(
    windows: &mut Vec<StudioEditExecutionWindow>,
    maximum_windows: usize,
    span: &crate::CompiledTimelineSpan,
    source_start: RationalTime,
    source_end: RationalTime,
    active: ActiveGaps,
) -> Result<(), StudioEditExecutionError> {
    if windows.len() == maximum_windows {
        return Err(StudioEditExecutionError::WindowLimitExceeded);
    }
    let start_offset = source_start
        .checked_sub(span.source_start)?
        .scaled(span.speed_denominator, span.speed_numerator)?;
    let end_offset = source_end
        .checked_sub(span.source_start)?
        .scaled(span.speed_denominator, span.speed_numerator)?;
    windows.push(StudioEditExecutionWindow {
        source_start,
        source_end,
        output_start: span.output_start.checked_add(start_offset)?,
        output_end: span.output_start.checked_add(end_offset)?,
        speed_numerator: span.speed_numerator,
        speed_denominator: span.speed_denominator,
        style: span.style,
        camera_source_available: active.camera == 0,
        microphone: StudioAudioExecution {
            source_available: active.microphone == 0,
            style: span.style.microphone,
        },
        system_audio: StudioAudioExecution {
            source_available: active.system_audio == 0,
            style: span.style.system_audio,
        },
    });
    Ok(())
}

fn compare_time_infallible(left: RationalTime, right: RationalTime) -> Ordering {
    (u128::from(left.ticks()) * u128::from(right.time_base().ticks_per_second()))
        .cmp(&(u128::from(right.ticks()) * u128::from(left.time_base().ticks_per_second())))
}

fn compare_duration(
    left: ExactDuration,
    right: ExactDuration,
) -> Result<Ordering, StudioEditExecutionError> {
    Ok(compare_fraction(
        left.numerator(),
        left.denominator(),
        right.numerator(),
        right.denominator(),
    ))
}

fn compare_fraction(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> Ordering {
    let mut reverse = false;
    loop {
        let whole_order =
            (left_numerator / left_denominator).cmp(&(right_numerator / right_denominator));
        if !whole_order.is_eq() {
            return if reverse {
                whole_order.reverse()
            } else {
                whole_order
            };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if reverse {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if reverse {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            (false, false) => {
                left_numerator = left_denominator;
                left_denominator = left_remainder;
                right_numerator = right_denominator;
                right_denominator = right_remainder;
                reverse = !reverse;
            }
        }
    }
}

fn duration_sub(
    left: ExactDuration,
    right: ExactDuration,
) -> Result<ExactDuration, StudioEditExecutionError> {
    if compare_duration(left, right)? == Ordering::Less {
        return Err(StudioError::TimelineUnderflow.into());
    }
    let common = gcd_u128(left.denominator(), right.denominator());
    let left_multiplier = right.denominator() / common;
    let right_multiplier = left.denominator() / common;
    let left_scaled = left
        .numerator()
        .checked_mul(left_multiplier)
        .ok_or(StudioError::TimelineOverflow)?;
    let right_scaled = right
        .numerator()
        .checked_mul(right_multiplier)
        .ok_or(StudioError::TimelineOverflow)?;
    let denominator = left
        .denominator()
        .checked_mul(left_multiplier)
        .ok_or(StudioError::TimelineOverflow)?;
    ExactDuration::new(left_scaled - right_scaled, denominator).map_err(Into::into)
}

const fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EditOperation, EditSpec, SourceCoverage, StudioTimelineCompiler, TimeBase, TimelineSource,
    };
    use std::collections::BTreeMap;

    fn time(seconds: u64) -> RationalTime {
        RationalTime::new(seconds, TimeBase::new(1).expect("one-second timebase"))
    }

    fn plan() -> CanonicalEditPlan {
        let source = TimelineSource {
            duration: time(10),
            coverage: vec![
                SourceCoverage {
                    track: TrackKind::Screen,
                    start: time(0),
                    end: time(10),
                },
                SourceCoverage {
                    track: TrackKind::Microphone,
                    start: time(0),
                    end: time(3),
                },
                SourceCoverage {
                    track: TrackKind::Microphone,
                    start: time(5),
                    end: time(10),
                },
            ],
            vfr_video_pts: BTreeMap::new(),
        };
        let edits = EditSpec {
            version: crate::STUDIO_EDIT_VERSION,
            revision: 2,
            operations: vec![
                EditOperation::Trim {
                    start: time(1),
                    end: time(9),
                },
                EditOperation::Speed {
                    start: time(2),
                    end: time(4),
                    numerator: 2,
                    denominator: 1,
                },
                EditOperation::DeleteRange {
                    start: time(6),
                    end: time(7),
                },
                EditOperation::AudioGain {
                    track: TrackKind::Microphone,
                    start: time(2),
                    end: time(4),
                    gain_millibels: -600,
                    muted: false,
                },
            ],
        };
        StudioTimelineCompiler::compile(&source, &edits).expect("canonical plan")
    }

    #[test]
    fn execution_windows_split_gaps_and_preserve_exact_timing_and_style() {
        let plan = plan();
        let executor = StudioEditExecutor::compile(
            &plan,
            MAX_STUDIO_EDIT_EXECUTION_WINDOWS,
            &CancellationToken::new(),
        )
        .expect("execution windows");

        assert_eq!(executor.plan_digest(), plan.digest());
        assert_eq!(
            executor.output_duration(),
            ExactDuration::new(6, 1).expect("six seconds")
        );
        assert_eq!(executor.windows().len(), 6);
        let fast_gap = executor
            .windows()
            .iter()
            .find(|window| window.source_start == time(3) && window.source_end == time(4))
            .expect("fast microphone gap");
        assert_eq!(fast_gap.playback_rate(), (2, 1));
        assert!(fast_gap.microphone.silent());
        assert_eq!(fast_gap.microphone.style.gain_millibels, -600);
        assert!(!fast_gap.camera_source_available);
        assert!(fast_gap.system_audio.silent());
        assert_eq!(
            fast_gap.output_start,
            ExactDuration::new(3, 2).expect("1.5 seconds")
        );
        assert_eq!(
            fast_gap.output_end,
            ExactDuration::new(2, 1).expect("two seconds")
        );
    }

    #[test]
    fn preview_lookup_and_export_batches_share_identical_windows() {
        let plan = plan();
        let cancellation = CancellationToken::new();
        let mut executor =
            StudioEditExecutor::compile(&plan, MAX_STUDIO_EDIT_EXECUTION_WINDOWS, &cancellation)
                .expect("execution windows");
        let preview = executor
            .window_at_output(
                ExactDuration::new(7, 4).expect("1.75 seconds"),
                &cancellation,
            )
            .expect("preview window");
        assert_eq!(
            preview
                .source_time_at_output(ExactDuration::new(7, 4).expect("1.75 seconds"))
                .expect("source point"),
            ExactDuration::new(7, 2).expect("3.5 seconds")
        );

        let expected = executor.windows().to_vec();
        let mut exported = Vec::new();
        while executor.remaining_windows() > 0 {
            exported.extend_from_slice(
                executor
                    .next_batch(2, &cancellation)
                    .expect("bounded export batch"),
            );
        }
        assert_eq!(exported, expected);
        assert!(exported.contains(&preview));
    }

    #[test]
    fn limits_and_cancellation_fail_before_advancing_the_cursor() {
        let plan = plan();
        assert_eq!(
            StudioEditExecutor::compile(&plan, 1, &CancellationToken::new()),
            Err(StudioEditExecutionError::WindowLimitExceeded)
        );

        let compile_cancel = CancellationToken::new();
        compile_cancel.cancel();
        assert_eq!(
            StudioEditExecutor::compile(&plan, MAX_STUDIO_EDIT_EXECUTION_WINDOWS, &compile_cancel,),
            Err(StudioEditExecutionError::Cancelled)
        );

        let mut executor = StudioEditExecutor::compile(
            &plan,
            MAX_STUDIO_EDIT_EXECUTION_WINDOWS,
            &CancellationToken::new(),
        )
        .expect("execution windows");
        let before = executor.remaining_windows();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            executor.next_batch(2, &cancellation),
            Err(StudioEditExecutionError::Cancelled)
        );
        assert_eq!(executor.remaining_windows(), before);
    }
}
