UPDATE auth_sessions_v2
SET state = 'revoked',
    revoked_at_ms = ?2,
    revocation_reason = 'logout_all',
    revision = revision + 1,
    last_operation_id = ?3
WHERE user_id = ?1 AND state = 'active';
