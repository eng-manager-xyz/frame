UPDATE auth_session_credentials_v2
SET state = 'revoked',
    revision = revision + 1,
    last_operation_id = ?3
WHERE session_id IN (
  SELECT id FROM auth_sessions_v2 WHERE user_id = ?1 AND state = 'active'
)
  AND state <> 'revoked';
