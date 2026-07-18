SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_sessions_v2 s
  JOIN auth_identities_v2 i ON i.user_id = s.user_id
  JOIN users u ON u.id = i.user_id AND u.status = 'active'
  JOIN auth_session_credentials_v2 c
    ON c.session_id = s.id
   AND c.key_version = ?2
   AND c.digest = ?3
   AND c.state = 'current'
  WHERE s.id = ?1
    AND s.state = 'active'
    AND s.session_version = i.session_version
    AND s.token_key_version = ?2
    AND s.token_digest = ?3
    AND (?4 IS NULL OR (s.csrf_key_version = ?4 AND s.csrf_digest = ?5))
    AND (
      ?6 = 'unchanged'
      OR (?6 = 'csrf_migrated' AND s.last_operation_id = ?7)
      OR (?6 = 'token_migrated' AND s.last_operation_id = ?7 AND c.last_operation_id = ?7)
    )
    AND (
      ?8 IS NULL
      OR EXISTS (
        SELECT 1 FROM auth_session_mutation_grants_v2 g
        WHERE g.id = ?8
          AND g.session_id = s.id
          AND g.user_id = s.user_id
          AND g.generation = s.generation
          AND g.token_key_version = ?2
          AND g.token_digest = ?3
          AND g.created_at_ms = ?9
          AND g.last_operation_id = ?7
      )
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?7
        AND a.action = ?10
        AND a.outcome = 'allow'
        AND a.reason = ?11
    )
) THEN 1 ELSE 0 END AS present
