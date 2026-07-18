SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_api_keys_v2 k
  JOIN users u ON u.id = k.owner_id AND u.status = 'active'
  WHERE k.id = ?1
    AND k.owner_id = ?2
    AND k.tenant_id = ?3
    AND k.key_version = ?4
    AND k.key_digest = ?5
    AND k.scopes_json = ?6
    AND k.created_at_ms = ?7
    AND ((?8 IS NULL AND k.expires_at_ms IS NULL) OR k.expires_at_ms = ?8)
    AND k.revoked_at_ms IS NULL
    AND k.last_operation_id = ?9
    AND NOT EXISTS (
      SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?10
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?9
        AND a.action = 'api_key_issue'
        AND a.outcome = 'allow'
        AND a.reason = 'issued'
    )
) THEN 1 ELSE 0 END AS present
