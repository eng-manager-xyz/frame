use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use thiserror::Error;

use crate::RuntimeDiagnostics;

pub const PIPELINE_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    Idle,
    Preparing,
    Running,
    Paused,
    Finalizing,
    Completed,
    Failed,
    Cancelled,
}

impl PipelineState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFaultCode {
    Negotiation,
    MissingFactory,
    SourceLost,
    SinkBlocked,
    Timeout,
    ResourceLimit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineFault {
    pub code: PipelineFaultCode,
    pub retryable: bool,
    pub safe_message: &'static str,
}

impl PipelineFault {
    #[must_use]
    pub const fn timeout() -> Self {
        Self {
            code: PipelineFaultCode::Timeout,
            retryable: true,
            safe_message: "pipeline deadline exceeded",
        }
    }

    #[must_use]
    pub const fn pipeline() -> Self {
        Self {
            code: PipelineFaultCode::Internal,
            retryable: false,
            safe_message: "pipeline reported an error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCommand {
    Prepare,
    Start,
    Pause,
    Resume,
    BeginFinalize,
    Complete,
    Fail(PipelineFault),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineTransition {
    pub sequence: u64,
    pub from: PipelineState,
    pub to: PipelineState,
    pub command: PipelineCommand,
}

#[derive(Debug, Clone)]
pub struct PipelineLifecycle {
    state: PipelineState,
    sequence: u64,
    terminal: Option<PipelineTransition>,
    last_fault: Option<PipelineFault>,
}

impl Default for PipelineLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineLifecycle {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: PipelineState::Idle,
            sequence: 0,
            terminal: None,
            last_fault: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> PipelineState {
        self.state
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn terminal_transition(&self) -> Option<PipelineTransition> {
        self.terminal
    }

    #[must_use]
    pub const fn last_fault(&self) -> Option<PipelineFault> {
        self.last_fault
    }

    pub fn apply(
        &mut self,
        command: PipelineCommand,
    ) -> Result<PipelineTransition, LifecycleError> {
        if let Some(terminal) = self.terminal {
            return Err(LifecycleError::AlreadyTerminal {
                terminal: terminal.to,
            });
        }

        let next = match (self.state, command) {
            (PipelineState::Idle, PipelineCommand::Prepare) => PipelineState::Preparing,
            (PipelineState::Preparing, PipelineCommand::Start) => PipelineState::Running,
            (PipelineState::Running, PipelineCommand::Pause) => PipelineState::Paused,
            (PipelineState::Paused, PipelineCommand::Resume) => PipelineState::Running,
            (PipelineState::Running | PipelineState::Paused, PipelineCommand::BeginFinalize) => {
                PipelineState::Finalizing
            }
            (PipelineState::Finalizing, PipelineCommand::Complete) => PipelineState::Completed,
            (
                PipelineState::Preparing
                | PipelineState::Running
                | PipelineState::Paused
                | PipelineState::Finalizing,
                PipelineCommand::Fail(fault),
            ) => {
                self.last_fault = Some(fault);
                PipelineState::Failed
            }
            (
                PipelineState::Idle
                | PipelineState::Preparing
                | PipelineState::Running
                | PipelineState::Paused
                | PipelineState::Finalizing,
                PipelineCommand::Cancel,
            ) => PipelineState::Cancelled,
            _ => {
                return Err(LifecycleError::InvalidTransition {
                    from: self.state,
                    command,
                });
            }
        };

        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(LifecycleError::SequenceOverflow)?;
        let transition = PipelineTransition {
            sequence: self.sequence,
            from: self.state,
            to: next,
            command,
        };
        self.state = next;
        if next.is_terminal() {
            self.terminal = Some(transition);
        }
        Ok(transition)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("cannot apply {command:?} while pipeline is {from:?}")]
    InvalidTransition {
        from: PipelineState,
        command: PipelineCommand,
    },
    #[error("pipeline already emitted terminal state {terminal:?}")]
    AlreadyTerminal { terminal: PipelineState },
    #[error("pipeline transition sequence overflow")]
    SequenceOverflow,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::AcqRel)
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    Block,
    DropOldest,
    DropNewest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePolicy {
    pub max_buffers: u32,
    pub max_bytes: u64,
    pub max_time_ns: u64,
    pub overflow: OverflowPolicy,
}

impl QueuePolicy {
    pub fn validate(self) -> Result<Self, PlanError> {
        if self.max_buffers == 0 && self.max_bytes == 0 && self.max_time_ns == 0 {
            return Err(PlanError::UnboundedQueue);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    pub memory_bytes: u64,
    pub deadline_ms: u64,
    pub max_output_bytes: u64,
}

impl ResourceLimits {
    pub fn validate(self) -> Result<Self, PlanError> {
        if self.memory_bytes == 0 || self.deadline_ms == 0 || self.max_output_bytes == 0 {
            return Err(PlanError::InvalidResourceLimit);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRole {
    Source,
    Queue,
    Convert,
    Filter,
    Encoder,
    Muxer,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedElement {
    pub factory: &'static str,
    pub role: ElementRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsConstraint {
    pub media_type: &'static str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

impl CapsConstraint {
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.media_type.trim().is_empty()
            || self.width == Some(0)
            || self.height == Some(0)
            || self.sample_rate == Some(0)
            || self.channels == Some(0)
        {
            return Err(PlanError::InvalidCaps);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinePlan {
    pub protocol_version: u16,
    pub elements: Vec<PlannedElement>,
    pub caps: Vec<CapsConstraint>,
    pub queue: QueuePolicy,
    pub resources: ResourceLimits,
}

impl PipelinePlan {
    pub fn validate(&self, runtime: &RuntimeDiagnostics) -> Result<(), PlanError> {
        if self.protocol_version != PIPELINE_PROTOCOL_VERSION {
            return Err(PlanError::UnsupportedProtocol(self.protocol_version));
        }
        if self.elements.is_empty() {
            return Err(PlanError::EmptyPipeline);
        }
        self.queue.validate()?;
        self.resources.validate()?;
        for caps in &self.caps {
            caps.validate()?;
        }
        for element in &self.elements {
            let declared = runtime
                .factories
                .iter()
                .find(|factory| factory.factory == element.factory)
                .ok_or(PlanError::UndeclaredFactory(element.factory))?;
            if !declared.available {
                return Err(PlanError::MissingFactory(element.factory));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    #[error("unsupported pipeline protocol version {0}")]
    UnsupportedProtocol(u16),
    #[error("pipeline has no elements")]
    EmptyPipeline,
    #[error("pipeline queue has no bound")]
    UnboundedQueue,
    #[error("pipeline resource limits must all be non-zero")]
    InvalidResourceLimit,
    #[error("pipeline has invalid caps")]
    InvalidCaps,
    #[error("required pipeline factory is unavailable: {0}")]
    MissingFactory(&'static str),
    #[error("pipeline factory is absent from the audited runtime manifest: {0}")]
    UndeclaredFactory(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_has_one_terminal_result() {
        let mut lifecycle = PipelineLifecycle::new();
        lifecycle.apply(PipelineCommand::Prepare).expect("prepare");
        lifecycle.apply(PipelineCommand::Start).expect("start");
        lifecycle.apply(PipelineCommand::Pause).expect("pause");
        lifecycle.apply(PipelineCommand::Resume).expect("resume");
        lifecycle
            .apply(PipelineCommand::BeginFinalize)
            .expect("finalize");
        let terminal = lifecycle
            .apply(PipelineCommand::Complete)
            .expect("complete");
        assert_eq!(terminal.to, PipelineState::Completed);
        assert!(matches!(
            lifecycle.apply(PipelineCommand::Cancel),
            Err(LifecycleError::AlreadyTerminal { .. })
        ));
        assert_eq!(lifecycle.terminal_transition(), Some(terminal));
    }

    #[test]
    fn invalid_transition_does_not_advance_sequence() {
        let mut lifecycle = PipelineLifecycle::new();
        assert!(lifecycle.apply(PipelineCommand::Start).is_err());
        assert_eq!(lifecycle.sequence(), 0);
        assert_eq!(lifecycle.state(), PipelineState::Idle);
    }

    #[test]
    fn cancellation_is_shared_and_idempotent() {
        let token = CancellationToken::new();
        let observer = token.clone();
        assert!(token.cancel());
        assert!(!token.cancel());
        assert!(observer.is_cancelled());
    }

    #[test]
    fn plans_reject_unbounded_queues() {
        let policy = QueuePolicy {
            max_buffers: 0,
            max_bytes: 0,
            max_time_ns: 0,
            overflow: OverflowPolicy::Block,
        };
        assert_eq!(policy.validate(), Err(PlanError::UnboundedQueue));
    }
}
