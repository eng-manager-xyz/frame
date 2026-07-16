SELECT i.session_version AS new_session_version,
       (
         SELECT COUNT(*) FROM auth_sessions_v2 changed
         WHERE changed.user_id = i.user_id
           AND changed.state = 'revoked'
           AND changed.revocation_reason = 'logout_all'
           AND changed.last_operation_id = ?2
       ) AS revoked_sessions
FROM auth_identities_v2 i
WHERE i.user_id = ?1
  AND i.last_operation_id = ?2
  AND NOT EXISTS (
    SELECT 1 FROM auth_sessions_v2 s
    WHERE s.user_id = i.user_id AND s.state = 'active'
  )
  AND NOT EXISTS (
    SELECT 1
    FROM auth_session_credentials_v2 c
    JOIN auth_sessions_v2 s ON s.id = c.session_id
    WHERE s.user_id = i.user_id AND c.state <> 'revoked'
  )
  AND NOT EXISTS (
    SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?3
  )
  AND EXISTS (
    SELECT 1 FROM auth_audit_events_v2 a
    WHERE a.operation_id = ?2
      AND a.action = 'logout_all'
      AND a.outcome = 'allow'
      AND a.reason = 'logged_out_all'
  )
LIMIT 1
