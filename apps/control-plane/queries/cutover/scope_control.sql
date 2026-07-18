UPDATE cutover_authority_scopes
SET replay_paused = ?6,
    epoch = ?4 + 1,
    audit_head = ?7,
    updated_at_ms = ?8
WHERE tenant_id = ?1
  AND domain = ?2
  AND phase = ?3
  AND epoch = ?4
  AND audit_head = ?5
