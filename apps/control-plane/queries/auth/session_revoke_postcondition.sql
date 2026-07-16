SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_sessions_v2 s
  WHERE s.id = ?1
    AND s.state = 'revoked'
    AND s.revoked_at_ms = ?2
    AND s.revocation_reason = ?3
    AND s.last_operation_id = ?4
    AND NOT EXISTS (
      SELECT 1 FROM auth_session_credentials_v2 c
      WHERE c.session_id = s.id AND c.state <> 'revoked'
    )
    AND NOT EXISTS (
      SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?5
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?4
        AND a.action = 'logout'
        AND a.outcome = 'allow'
        AND a.reason = 'logged_out'
    )
) THEN 1 ELSE 0 END AS present
