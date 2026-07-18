INSERT INTO cutover_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM cutover_authority_scopes AS authority
         WHERE authority.tenant_id = ?2
           AND authority.domain = ?3
           AND authority.epoch = ?4
           AND authority.audit_head = ?5
           AND authority.phase IN ('shadow_read', 'dual_write', 'd1_authoritative', 'rolled_back')
           AND ?7 >= authority.updated_at_ms
           AND (
             (?6 = 'pause' AND authority.replay_paused = 0)
             OR (?6 = 'resume' AND authority.replay_paused = 1)
           )
           AND (
             ?8 = 0 OR EXISTS (
               SELECT 1 FROM cutover_maintenance_windows AS window
               WHERE window.tenant_id = authority.tenant_id
                 AND window.domain = authority.domain
                 AND ?7 BETWEEN window.starts_at_ms AND window.ends_at_ms
             )
           )
       ) THEN 1 ELSE 0 END
