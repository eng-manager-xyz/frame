INSERT INTO cutover_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM cutover_authority_scopes AS authority
         WHERE authority.tenant_id = ?2
           AND authority.domain = ?3
           AND authority.phase = ?4
           AND authority.writer = ?5
           AND authority.replay_paused = ?6
           AND authority.epoch = ?7
           AND authority.audit_head = ?8
           AND authority.updated_at_ms = ?9
           AND authority.phase_epoch = ?10
       ) THEN 1 ELSE 0 END
