use frame_domain::{CutoverEvidence, CutoverPhase, CutoverState};
use frame_ports::MigrationStateRepository;

use crate::ApplicationError;

pub struct CutoverCoordinator<'a> {
    repository: &'a dyn MigrationStateRepository,
}

impl<'a> CutoverCoordinator<'a> {
    #[must_use]
    pub const fn new(repository: &'a dyn MigrationStateRepository) -> Self {
        Self { repository }
    }

    pub async fn transition(
        &self,
        next: CutoverPhase,
        evidence: CutoverEvidence,
    ) -> Result<CutoverState, ApplicationError> {
        let current = self.repository.cutover_state().await?;
        let mut proposed = current;
        proposed
            .transition(next, evidence)
            .map_err(|_| ApplicationError::Invalid)?;
        self.repository
            .compare_and_set_cutover(current.epoch, proposed)
            .await?;
        Ok(proposed)
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{CutoverEvidence, CutoverPhase, DataAuthority};
    use frame_ports::{MemoryMigrationStateRepository, MigrationStateRepository};

    use super::*;

    #[tokio::test]
    async fn cutover_requires_evidence_and_changes_one_authority_at_a_time() {
        let repository = MemoryMigrationStateRepository::default();
        let coordinator = CutoverCoordinator::new(&repository);
        let shadow = coordinator
            .transition(CutoverPhase::ShadowRead, CutoverEvidence::default())
            .await
            .expect("shadow");
        assert_eq!(shadow.authority, DataAuthority::Legacy);
        assert_eq!(
            coordinator
                .transition(CutoverPhase::DualWrite, CutoverEvidence::default())
                .await,
            Err(ApplicationError::Invalid)
        );
        let dual = coordinator
            .transition(
                CutoverPhase::DualWrite,
                CutoverEvidence {
                    rollback_rehearsed: true,
                    ..CutoverEvidence::default()
                },
            )
            .await
            .expect("dual write");
        assert_eq!(dual.authority, DataAuthority::DualWrite);
        let d1 = coordinator
            .transition(
                CutoverPhase::D1Authoritative,
                CutoverEvidence {
                    reconciliation_clean: true,
                    rollback_rehearsed: true,
                    observation_window_complete: false,
                },
            )
            .await
            .expect("D1 authority");
        assert_eq!(d1.authority, DataAuthority::D1);
        assert_eq!(d1.epoch, 3);
    }

    #[tokio::test]
    async fn repository_compare_and_set_fences_stale_operator() {
        let repository = MemoryMigrationStateRepository::default();
        let stale = repository.cutover_state().await.expect("state");
        CutoverCoordinator::new(&repository)
            .transition(CutoverPhase::ShadowRead, CutoverEvidence::default())
            .await
            .expect("advance");
        let mut conflicting = stale;
        conflicting
            .transition(CutoverPhase::ShadowRead, CutoverEvidence::default())
            .expect("proposal");
        assert_eq!(
            repository
                .compare_and_set_cutover(stale.epoch, conflicting)
                .await,
            Err(frame_ports::PortError::Conflict)
        );
    }
}
