INSERT INTO cutover_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM cutover_authority_scopes AS authority
         JOIN cutover_slo_config AS policy
           ON policy.tenant_id = authority.tenant_id
          AND policy.domain = authority.domain
         WHERE authority.tenant_id = ?2
           AND authority.domain = ?3
           AND authority.epoch = ?4
           AND authority.audit_head = ?5
           AND ?7 >= authority.updated_at_ms
           AND policy.updated_at_ms <= ?7
           AND (
             (?6 = 'shadow_read' AND authority.phase = 'legacy_authoritative')
             OR (?6 = 'dual_write' AND authority.phase IN ('shadow_read', 'rolled_back'))
             OR (?6 = 'd1_authoritative' AND authority.phase = 'dual_write')
             OR (?6 = 'rolled_back' AND authority.phase = 'd1_authoritative')
           )
           AND (
             ?8 = 0 OR EXISTS (
               SELECT 1 FROM cutover_maintenance_windows AS window
               WHERE window.tenant_id = authority.tenant_id
                 AND window.domain = authority.domain
                 AND ?7 BETWEEN window.starts_at_ms AND window.ends_at_ms
             )
           )
           AND (
             ?9 = 0 OR (
               (SELECT COUNT(*) FROM cutover_shadow_query_requirements AS requirement
                WHERE requirement.tenant_id = authority.tenant_id
                  AND requirement.domain = authority.domain) > 0
               AND NOT EXISTS (
                 SELECT 1 FROM cutover_shadow_query_requirements AS requirement
                 WHERE requirement.tenant_id = authority.tenant_id
                   AND requirement.domain = authority.domain
                   AND (SELECT COUNT(*) FROM cutover_shadow_observations AS observation
                        WHERE observation.tenant_id = requirement.tenant_id
                          AND observation.domain = requirement.domain
                          AND observation.phase_epoch = authority.phase_epoch
                          AND observation.query_class = requirement.query_class
                          AND observation.normalization_digest = requirement.normalization_digest
                          AND observation.observed_at_ms BETWEEN
                            MAX(0, ?7 - policy.shadow_window_ms, authority.phase_started_at_ms)
                            AND ?7) < policy.minimum_shadow_observations
               )
               AND NOT EXISTS (
                 SELECT 1 FROM cutover_shadow_observations AS observation
                 JOIN cutover_shadow_query_requirements AS requirement
                   ON requirement.tenant_id = observation.tenant_id
                  AND requirement.domain = observation.domain
                  AND requirement.query_class = observation.query_class
                  AND requirement.normalization_digest = observation.normalization_digest
                 WHERE observation.tenant_id = authority.tenant_id
                   AND observation.domain = authority.domain
                   AND observation.phase_epoch = authority.phase_epoch
                   AND observation.observed_at_ms BETWEEN
                     MAX(0, ?7 - policy.shadow_window_ms, authority.phase_started_at_ms) AND ?7
                   AND observation.classification IN ('semantic_mismatch', 'missing', 'error')
               )
               AND COALESCE((
                 SELECT MAX(0, ?7 - MIN(event.occurred_at_ms))
                 FROM cutover_change_events AS event
                 WHERE event.tenant_id = authority.tenant_id
                   AND event.domain = authority.domain
                   AND event.state = 'pending'
               ), 0) <= policy.max_pending_lag_ms
               AND (SELECT COUNT(*) FROM cutover_change_events AS event
                    WHERE event.tenant_id = authority.tenant_id
                      AND event.domain = authority.domain
                      AND event.state = 'dead_letter') <= policy.max_dead_letter_events
               AND (SELECT COUNT(*) FROM cutover_operational_signal_events AS signal
                    WHERE signal.tenant_id = authority.tenant_id
                      AND signal.domain = authority.domain
                      AND signal.phase_epoch = authority.phase_epoch
                      AND signal.kind = 'authority_contention'
                      AND signal.occurred_at_ms BETWEEN
                        MAX(0, ?7 - policy.shadow_window_ms, authority.phase_started_at_ms)
                        AND ?7) <= policy.max_contention_events
               AND NOT EXISTS (
                 SELECT 1 FROM cutover_operational_signal_events AS signal
                 WHERE signal.tenant_id = authority.tenant_id
                   AND signal.domain = authority.domain
                   AND signal.phase_epoch = authority.phase_epoch
                   AND signal.kind IN ('replay_write_failure', 'replay_lost_ack')
                   AND signal.occurred_at_ms BETWEEN
                     MAX(0, ?7 - policy.shadow_window_ms, authority.phase_started_at_ms) AND ?7
               )
             )
           )
           AND (
             ?10 = 0 OR NOT EXISTS (
               SELECT 1 FROM cutover_change_events AS event
               WHERE event.tenant_id = authority.tenant_id
                 AND event.domain = authority.domain
                 AND event.state IN ('pending', 'dead_letter')
             )
           )
           AND NOT (
             ?6 IN ('shadow_read', 'dual_write', 'rolled_back')
             AND EXISTS (
               SELECT 1 FROM authority_state AS singleton
               WHERE singleton.singleton = 1 AND singleton.authority = 'd1'
             )
           )
       ) THEN 1 ELSE 0 END
