SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_sessions_v2 s
  JOIN auth_session_credentials_v2 c
    ON c.session_id = s.id
   AND c.key_version = s.token_key_version
   AND c.digest = s.token_digest
  JOIN auth_identities_v2 i ON i.user_id = s.user_id
  JOIN users u ON u.id = s.user_id AND u.status = 'active'
  WHERE s.id = ?1
    AND s.family_id = ?2
    AND s.user_id = ?3
    AND s.client_kind = ?4
    AND s.token_key_version = ?5
    AND s.token_digest = ?6
    AND ((?7 IS NULL AND s.csrf_key_version IS NULL) OR s.csrf_key_version = ?7)
    AND ((?8 IS NULL AND s.csrf_digest IS NULL) OR s.csrf_digest = ?8)
    AND ((?9 IS NULL AND s.browser_origin IS NULL) OR s.browser_origin = ?9)
    AND s.issued_at_ms = ?10
    AND s.rotated_at_ms = ?10
    AND s.idle_expires_at_ms = ?11
    AND s.absolute_expires_at_ms = ?12
    AND s.session_version = i.session_version
    AND s.generation = 0
    AND s.state = 'active'
    AND s.last_operation_id = ?13
    AND c.family_id = s.family_id
    AND c.state = 'current'
    AND c.last_operation_id = ?13
    AND (
      (?14 = 'issuance' AND NOT EXISTS (
        SELECT 1 FROM auth_principal_issuance_grants_v2 g WHERE g.id = ?15
      ))
      OR
      (?14 = 'mutation' AND NOT EXISTS (
        SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?15
      ))
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?13
        AND a.action = 'session_issue'
        AND a.outcome = 'allow'
        AND a.reason = 'issued'
    )
) THEN 1 ELSE 0 END AS present
