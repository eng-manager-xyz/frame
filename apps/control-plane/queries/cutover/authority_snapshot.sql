WITH scoped AS (
  SELECT *
  FROM cutover_authority_scopes
  WHERE tenant_id = ?1 AND domain = ?2
), policy AS (
  SELECT *
  FROM cutover_slo_config
  WHERE tenant_id = ?1 AND domain = ?2
), windowed AS (
  SELECT MAX(0, ?3 - policy.shadow_window_ms, scoped.phase_started_at_ms) AS starts_at_ms
  FROM scoped CROSS JOIN policy
)
SELECT
  scoped.tenant_id,
  scoped.domain,
  scoped.phase,
  scoped.writer,
  scoped.mirror_enabled,
  scoped.replay_paused,
  scoped.epoch,
  scoped.phase_epoch,
  scoped.audit_head,
  scoped.rollback_ready,
  scoped.phase_started_at_ms,
  scoped.updated_at_ms,
  singleton.phase AS singleton_phase,
  singleton.authority AS singleton_authority,
  singleton.epoch AS singleton_epoch,
  singleton.updated_at_ms AS singleton_updated_at_ms,
  policy.shadow_window_ms,
  policy.minimum_shadow_observations,
  policy.max_pending_lag_ms,
  policy.max_shadow_mismatches,
  policy.max_dead_letter_events,
  policy.max_contention_events,
  policy.approved_by_digest,
  policy.updated_at_ms AS slo_updated_at_ms,
  windowed.starts_at_ms AS observation_window_started_at_ms,
  (SELECT COUNT(*)
   FROM cutover_shadow_query_requirements AS requirement
   WHERE requirement.tenant_id = scoped.tenant_id
     AND requirement.domain = scoped.domain) AS required_query_classes,
  (SELECT COUNT(*)
   FROM cutover_shadow_query_requirements AS requirement
   WHERE requirement.tenant_id = scoped.tenant_id
     AND requirement.domain = scoped.domain
     AND (SELECT COUNT(*)
          FROM cutover_shadow_observations AS observation
          WHERE observation.tenant_id = requirement.tenant_id
            AND observation.domain = requirement.domain
            AND observation.phase_epoch = scoped.phase_epoch
            AND observation.query_class = requirement.query_class
            AND observation.normalization_digest = requirement.normalization_digest
            AND observation.observed_at_ms BETWEEN windowed.starts_at_ms AND ?3)
       >= policy.minimum_shadow_observations) AS covered_query_classes,
  (SELECT COUNT(*)
   FROM cutover_shadow_observations AS observation
   JOIN cutover_shadow_query_requirements AS requirement
     ON requirement.tenant_id = observation.tenant_id
    AND requirement.domain = observation.domain
    AND requirement.query_class = observation.query_class
    AND requirement.normalization_digest = observation.normalization_digest
   WHERE observation.tenant_id = scoped.tenant_id
     AND observation.domain = scoped.domain
     AND observation.phase_epoch = scoped.phase_epoch
     AND observation.observed_at_ms BETWEEN windowed.starts_at_ms AND ?3)
    AS shadow_observations,
  (SELECT COUNT(*)
   FROM cutover_shadow_observations AS observation
   JOIN cutover_shadow_query_requirements AS requirement
     ON requirement.tenant_id = observation.tenant_id
    AND requirement.domain = observation.domain
    AND requirement.query_class = observation.query_class
    AND requirement.normalization_digest = observation.normalization_digest
   WHERE observation.tenant_id = scoped.tenant_id
     AND observation.domain = scoped.domain
     AND observation.phase_epoch = scoped.phase_epoch
     AND observation.observed_at_ms BETWEEN windowed.starts_at_ms AND ?3
     AND observation.classification IN ('semantic_mismatch', 'missing', 'error'))
    AS shadow_mismatches,
  (SELECT COUNT(*) FROM cutover_change_events AS event
   WHERE event.tenant_id = scoped.tenant_id AND event.domain = scoped.domain
     AND event.state = 'pending') AS pending_events,
  (SELECT COUNT(*) FROM cutover_change_events AS event
   WHERE event.tenant_id = scoped.tenant_id AND event.domain = scoped.domain
     AND event.state = 'dead_letter') AS dead_letter_events,
  COALESCE((SELECT MAX(0, ?3 - MIN(event.occurred_at_ms))
            FROM cutover_change_events AS event
            WHERE event.tenant_id = scoped.tenant_id AND event.domain = scoped.domain
              AND event.state = 'pending'), 0) AS pending_lag_ms,
  (SELECT COUNT(*) FROM cutover_operational_signal_events AS signal
   WHERE signal.tenant_id = scoped.tenant_id AND signal.domain = scoped.domain
     AND signal.phase_epoch = scoped.phase_epoch
     AND signal.kind = 'authority_contention'
     AND signal.occurred_at_ms BETWEEN windowed.starts_at_ms AND ?3)
    AS authority_contention_events,
  (SELECT COUNT(*) FROM cutover_operational_signal_events AS signal
   WHERE signal.tenant_id = scoped.tenant_id AND signal.domain = scoped.domain
     AND signal.phase_epoch = scoped.phase_epoch
     AND signal.kind = 'replay_write_failure'
     AND signal.occurred_at_ms BETWEEN windowed.starts_at_ms AND ?3)
    AS replay_write_failures,
  (SELECT COUNT(*) FROM cutover_operational_signal_events AS signal
   WHERE signal.tenant_id = scoped.tenant_id AND signal.domain = scoped.domain
     AND signal.phase_epoch = scoped.phase_epoch
     AND signal.kind = 'replay_lost_ack'
     AND signal.occurred_at_ms BETWEEN windowed.starts_at_ms AND ?3)
    AS replay_lost_ack_events,
  CASE WHEN NOT EXISTS (
    SELECT 1 FROM cutover_operational_signals AS rollup
    WHERE rollup.tenant_id = scoped.tenant_id AND rollup.domain = scoped.domain
      AND (
        rollup.count <> (SELECT COUNT(*) FROM cutover_operational_signal_events AS signal
                         WHERE signal.tenant_id = rollup.tenant_id
                           AND signal.domain = rollup.domain
                           AND signal.kind = rollup.kind)
        OR rollup.last_at_ms <> (SELECT MAX(signal.occurred_at_ms)
                                 FROM cutover_operational_signal_events AS signal
                                 WHERE signal.tenant_id = rollup.tenant_id
                                   AND signal.domain = rollup.domain
                                   AND signal.kind = rollup.kind)
      )
  ) AND NOT EXISTS (
    SELECT 1 FROM cutover_operational_signal_events AS signal
    WHERE signal.tenant_id = scoped.tenant_id AND signal.domain = scoped.domain
      AND NOT EXISTS (
        SELECT 1 FROM cutover_operational_signals AS rollup
        WHERE rollup.tenant_id = signal.tenant_id
          AND rollup.domain = signal.domain
          AND rollup.kind = signal.kind
      )
  ) THEN 1 ELSE 0 END AS signal_rollup_consistent,
  CASE WHEN scoped.epoch = 0 THEN scoped.audit_head = lower(hex(zeroblob(32)))
       ELSE EXISTS (
         SELECT 1 FROM cutover_authority_audit AS audit
         WHERE audit.audit_hash = scoped.audit_head
           AND audit.tenant_id = scoped.tenant_id
           AND audit.domain = scoped.domain
           AND audit.to_epoch = scoped.epoch
           AND audit.to_phase = scoped.phase
           AND audit.occurred_at_ms = scoped.updated_at_ms
       ) END AS audit_head_consistent,
  CASE WHEN EXISTS (
    SELECT 1 FROM cutover_maintenance_windows AS window
    WHERE window.tenant_id = scoped.tenant_id
      AND window.domain = scoped.domain
      AND ?3 BETWEEN window.starts_at_ms AND window.ends_at_ms
  ) THEN 1 ELSE 0 END AS maintenance_window_active
FROM scoped
CROSS JOIN policy
CROSS JOIN windowed
JOIN authority_state AS singleton ON singleton.singleton = 1
