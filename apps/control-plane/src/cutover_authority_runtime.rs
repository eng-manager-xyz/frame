//! Runtime composition boundary for Issue-17 cutover controls.
//!
//! Keeping this facade isolated lets the integrated Worker routes share the
//! exact D1 authority adapter without leaking Worker bindings into domain or
//! application contracts.

use frame_domain::{CutoverScope, TimestampMillis};
use worker::D1Database;

use crate::cutover_authority::{
    ApprovedCutoverTransition, ApprovedReplayControl, CutoverAuthorityResult,
    CutoverAuthoritySnapshot, CutoverShadowObservation, CutoverSignalKind,
    D1CutoverAuthorityRepository,
};

pub struct CutoverAuthorityRuntime<'database> {
    repository: D1CutoverAuthorityRepository<'database>,
}

impl<'database> CutoverAuthorityRuntime<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self {
            repository: D1CutoverAuthorityRepository::new(database),
        }
    }

    #[must_use]
    pub const fn repository(&self) -> &D1CutoverAuthorityRepository<'database> {
        &self.repository
    }

    pub async fn status(
        &self,
        scope: &CutoverScope,
        observed_at: TimestampMillis,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        self.repository.snapshot(scope, observed_at).await
    }

    pub async fn transition(
        &self,
        command: &ApprovedCutoverTransition,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        self.repository.transition(command).await
    }

    pub async fn replay_control(
        &self,
        command: &ApprovedReplayControl,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        self.repository.replay_control(command).await
    }

    pub async fn record_signal(
        &self,
        scope: &CutoverScope,
        expected_phase_epoch: u64,
        kind: CutoverSignalKind,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<()> {
        self.repository
            .record_signal(scope, expected_phase_epoch, kind, occurred_at)
            .await
    }

    pub async fn record_shadow_observation(
        &self,
        observation: &CutoverShadowObservation,
    ) -> CutoverAuthorityResult<()> {
        self.repository.record_shadow_observation(observation).await
    }
}

/// Router and mutation groups bound to this runtime by the Worker.
pub const INTEGRATED_ROUTE_GROUPS: [&str; 4] = [
    "cutover-authority-status",
    "cutover-transition-pause-resume",
    "cutover-shadow-signal-ingest",
    "scoped-writer-fence-for-every-d1-mutation",
];

#[cfg(test)]
mod tests {
    use super::INTEGRATED_ROUTE_GROUPS;

    #[test]
    fn route_inventory_is_closed_and_unique() {
        let mut groups = INTEGRATED_ROUTE_GROUPS;
        groups.sort_unstable();
        assert!(groups.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
