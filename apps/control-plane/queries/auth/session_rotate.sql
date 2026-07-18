UPDATE auth_sessions_v2
SET token_key_version = ?4,
    token_digest = ?5,
    csrf_key_version = ?6,
    csrf_digest = ?7,
    rotated_at_ms = ?8,
    idle_expires_at_ms = ?9,
    generation = generation + 1,
    revision = revision + 1,
    last_operation_id = ?10
WHERE id = ?1
  AND revision = ?2
  AND generation = ?3
  AND state = 'active'
  AND rotated_at_ms <= ?8
  AND absolute_expires_at_ms > ?8
  AND ?9 > ?8
  AND ?9 <= absolute_expires_at_ms
