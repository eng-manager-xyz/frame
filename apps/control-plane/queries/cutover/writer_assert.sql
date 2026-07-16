INSERT INTO cutover_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM cutover_authority_scopes AS authority
         WHERE authority.tenant_id = ?2
           AND authority.domain = ?3
           AND authority.writer = ?4
           AND authority.epoch = ?5
           AND ?6 >= authority.updated_at_ms
           AND (
             (?4 = 'legacy' AND authority.mirror_enabled = 1
               AND authority.phase IN ('dual_write', 'rolled_back'))
             OR (?4 = 'd1' AND authority.phase IN ('d1_authoritative', 'finalized'))
           )
           AND NOT (
             ?4 = 'legacy'
             AND EXISTS (
               SELECT 1 FROM authority_state AS singleton
               WHERE singleton.singleton = 1 AND singleton.authority = 'd1'
             )
           )
       ) THEN 1 ELSE 0 END
