UPDATE cutover_authority_scopes
SET phase = ?6,
    writer = ?7,
    mirror_enabled = ?8,
    replay_paused = 0,
    epoch = ?4 + 1,
    phase_epoch = ?4 + 1,
    audit_head = ?9,
    rollback_ready = ?10,
    phase_started_at_ms = ?11,
    updated_at_ms = ?11
WHERE tenant_id = ?1
  AND domain = ?2
  AND epoch = ?4
  AND phase = ?3
  AND audit_head = ?5
