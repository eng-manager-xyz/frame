UPDATE auth_sessions_v2
SET state = 'revoked',
    revoked_at_ms = ?2,
    revocation_reason = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE family_id = ?1
  AND state = 'active'
