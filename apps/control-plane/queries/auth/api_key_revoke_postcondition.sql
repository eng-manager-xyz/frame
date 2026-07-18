SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_api_keys_v2 k
  WHERE k.id = ?1
    AND k.revoked_at_ms IS NOT NULL
    AND NOT EXISTS (
      SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?3
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?2
        AND a.action = 'api_key_revoke'
        AND a.outcome = 'allow'
        AND a.reason = 'revoked'
    )
) THEN 1 ELSE 0 END AS present
