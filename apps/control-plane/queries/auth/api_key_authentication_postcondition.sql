SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_api_keys_v2 k
  JOIN auth_identities_v2 i ON i.user_id = k.owner_id
  JOIN users u ON u.id = i.user_id AND u.status = 'active'
  JOIN organization_members m
    ON m.user_id = i.user_id
   AND m.organization_id = k.tenant_id
   AND m.state = 'active'
  JOIN organizations o
    ON o.id = m.organization_id
   AND o.status = 'active'
  WHERE k.id = ?1
    AND k.tenant_id = ?2
    AND k.key_version = ?3
    AND k.key_digest = ?4
    AND k.revoked_at_ms IS NULL
    AND (?5 = 'unchanged' OR (?5 = 'migrated' AND k.last_operation_id = ?6))
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?6
        AND a.action = 'api_key_authenticate'
        AND a.outcome = 'allow'
        AND a.reason = ?7
    )
) THEN 1 ELSE 0 END AS present
