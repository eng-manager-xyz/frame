SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_verification_challenges_v2 v
  JOIN auth_identities_v2 i ON i.user_id = v.user_id
  JOIN users u ON u.id = i.user_id AND u.status = 'active'
  JOIN auth_principal_issuance_grants_v2 g
    ON g.id = ?10
   AND g.user_id = i.user_id
   AND g.identity_revision = i.identity_revision
   AND g.expires_at_ms = v.expires_at_ms
   AND g.created_at_ms = ?8
   AND g.last_operation_id = ?9
  WHERE v.id = ?1
    AND v.user_id = ?2
    AND v.identifier_key_version = ?3
    AND v.identifier_digest = ?4
    AND v.secret_key_version = ?5
    AND v.secret_digest = ?6
    AND v.purpose = ?7
    AND v.state = 'consumed'
    AND v.consumed_at_ms = ?8
    AND v.last_operation_id = ?9
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?9
        AND a.action = 'verification_consume'
        AND a.outcome = 'allow'
        AND a.reason = 'verification_completed'
    )
    AND (
      v.purpose <> 'account_recovery'
      OR (
        i.last_operation_id = ?9
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
      )
    )
) THEN 1 ELSE 0 END AS present
