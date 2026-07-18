use frame_domain::{
    AuthorityFence, CutoverEvidence, CutoverPhase, CutoverScope, CutoverState, DataAuthority,
};
use frame_ports::MigrationStateRepository;

use crate::ApplicationError;

pub struct CutoverCoordinator<'a> {
    repository: &'a dyn MigrationStateRepository,
    scope: CutoverScope,
}

impl<'a> CutoverCoordinator<'a> {
    #[must_use]
    pub const fn new(repository: &'a dyn MigrationStateRepository, scope: CutoverScope) -> Self {
        Self { repository, scope }
    }

    pub async fn transition(
        &self,
        next: CutoverPhase,
        evidence: CutoverEvidence,
    ) -> Result<CutoverState, ApplicationError> {
        let current = self.repository.cutover_state(&self.scope).await?;
        let mut proposed = current.clone();
        proposed
            .transition(next, evidence)
            .map_err(|_| ApplicationError::Invalid)?;
        self.repository
            .compare_and_set_cutover(&self.scope, current.epoch, proposed.clone())
            .await?;
        Ok(proposed)
    }

    pub async fn set_replay_paused(&self, paused: bool) -> Result<CutoverState, ApplicationError> {
        let current = self.repository.cutover_state(&self.scope).await?;
        let mut proposed = current.clone();
        proposed
            .set_replay_paused(paused)
            .map_err(|_| ApplicationError::Invalid)?;
        self.repository
            .compare_and_set_cutover(&self.scope, current.epoch, proposed.clone())
            .await?;
        Ok(proposed)
    }

    pub async fn authorize_writer(
        &self,
        writer: DataAuthority,
        expected_epoch: u64,
    ) -> Result<AuthorityFence, ApplicationError> {
        self.repository
            .cutover_state(&self.scope)
            .await?
            .authorize_writer(writer, expected_epoch)
            .map_err(|_| ApplicationError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{CutoverDomain, CutoverEvidence, CutoverPhase, DataAuthority, TenantId};
    use frame_ports::{MemoryMigrationStateRepository, MigrationStateRepository};

    use super::*;

    fn scope(tenant: &str) -> CutoverScope {
        CutoverScope::new(
            TenantId::parse(tenant).expect("tenant"),
            CutoverDomain::parse("metadata").expect("domain"),
        )
    }

    #[tokio::test]
    async fn cutover_requires_evidence_and_changes_one_authority_at_a_time() {
        let repository = MemoryMigrationStateRepository::default();
        let coordinator =
            CutoverCoordinator::new(&repository, scope("00000000-0000-0000-0000-000000000017"));
        let shadow = coordinator
            .transition(
                CutoverPhase::ShadowRead,
                CutoverEvidence {
                    shadow_observation_ready: true,
                    ..CutoverEvidence::default()
                },
            )
            .await
            .expect("shadow");
        assert_eq!(shadow.writer, DataAuthority::Legacy);
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
                    reconciliation_clean: true,
                    observation_window_complete: true,
                    ..CutoverEvidence::default()
                },
            )
            .await
            .expect("dual write");
        assert_eq!(dual.writer, DataAuthority::Legacy);
        assert!(dual.mirror_enabled);
        let paused = coordinator
            .set_replay_paused(true)
            .await
            .expect("pause replay");
        assert!(paused.replay_paused);
        coordinator
            .set_replay_paused(false)
            .await
            .expect("resume replay");
        let d1 = coordinator
            .transition(
                CutoverPhase::D1Authoritative,
                CutoverEvidence {
                    reconciliation_clean: true,
                    rollback_rehearsed: true,
                    observation_window_complete: true,
                    reconciliation_digest_present: true,
                    legacy_fenced: true,
                    ..CutoverEvidence::default()
                },
            )
            .await
            .expect("D1 authority");
        assert_eq!(d1.writer, DataAuthority::D1);
        assert!(d1.mirror_enabled);
        assert_eq!(d1.epoch, 5);
        assert!(
            coordinator
                .authorize_writer(DataAuthority::D1, d1.epoch)
                .await
                .is_ok()
        );
        assert_eq!(
            coordinator
                .authorize_writer(DataAuthority::Legacy, d1.epoch)
                .await,
            Err(ApplicationError::Invalid)
        );
    }

    #[tokio::test]
    async fn repository_compare_and_set_fences_stale_operator() {
        let repository = MemoryMigrationStateRepository::default();
        let scope = scope("00000000-0000-0000-0000-000000000017");
        let stale = repository.cutover_state(&scope).await.expect("state");
        CutoverCoordinator::new(&repository, scope.clone())
            .transition(
                CutoverPhase::ShadowRead,
                CutoverEvidence {
                    shadow_observation_ready: true,
                    ..CutoverEvidence::default()
                },
            )
            .await
            .expect("advance");
        let mut conflicting = stale.clone();
        conflicting
            .transition(
                CutoverPhase::ShadowRead,
                CutoverEvidence {
                    shadow_observation_ready: true,
                    ..CutoverEvidence::default()
                },
            )
            .expect("proposal");
        assert_eq!(
            repository
                .compare_and_set_cutover(&scope, stale.epoch, conflicting)
                .await,
            Err(frame_ports::PortError::Conflict)
        );
    }
}
