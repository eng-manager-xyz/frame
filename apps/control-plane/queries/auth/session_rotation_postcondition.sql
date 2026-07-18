SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_sessions_v2 s
  JOIN auth_identities_v2 i ON i.user_id = s.user_id
  JOIN users u ON u.id = s.user_id AND u.status = 'active'
  WHERE s.id = ?1
    AND s.generation = ?2
    AND s.token_key_version = ?3
    AND s.token_digest = ?4
    AND ((?5 IS NULL AND s.csrf_key_version IS NULL) OR s.csrf_key_version = ?5)
    AND ((?6 IS NULL AND s.csrf_digest IS NULL) OR s.csrf_digest = ?6)
    AND s.session_version = i.session_version
    AND s.state = 'active'
    AND s.last_operation_id = ?7
    AND EXISTS (
      SELECT 1 FROM auth_session_credentials_v2 old
      WHERE old.key_version = ?8
        AND old.digest = ?9
        AND old.session_id = s.id
        AND old.state = 'rotated'
        AND old.last_operation_id = ?7
    )
    AND EXISTS (
      SELECT 1 FROM auth_session_credentials_v2 current
      WHERE current.key_version = ?3
        AND current.digest = ?4
        AND current.session_id = s.id
        AND current.state = 'current'
        AND current.last_operation_id = ?7
    )
    AND NOT EXISTS (
      SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?10
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?7
        AND a.action = 'session_rotate'
        AND a.outcome = 'allow'
        AND a.reason = 'rotated'
    )
) THEN 1 ELSE 0 END AS present
