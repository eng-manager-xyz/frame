UPDATE auth_session_credentials_v2
SET state = 'revoked',
    revision = revision + 1,
    last_operation_id = ?2
WHERE session_id = ?1
  AND state <> 'revoked'
