UPDATE auth_sessions_v2
SET state = 'revoked',
    revoked_at_ms = ?4,
    revocation_reason = ?5,
    revision = revision + 1,
    last_operation_id = ?6
WHERE id = ?1
  AND revision = ?2
  AND generation = ?3
  AND state = 'active'
