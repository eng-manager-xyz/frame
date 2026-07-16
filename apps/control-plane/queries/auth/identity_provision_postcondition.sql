SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM users u
  JOIN auth_identities_v2 i ON i.user_id = u.id
  JOIN auth_identifier_digests_v2 d ON d.user_id = u.id
  WHERE u.id = ?1
    AND u.status = 'active'
    AND i.identity_revision = ?2
    AND i.last_operation_id = ?3
    AND d.key_version = ?4
    AND d.digest = ?5
    AND d.last_operation_id = ?3
    AND NOT EXISTS (
      SELECT 1 FROM auth_identity_provisioning_grants_v2 g WHERE g.id = ?6
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?3
        AND a.action = 'identity_provision'
        AND a.outcome = 'allow'
        AND a.reason = 'issued'
    )
) THEN 1 ELSE 0 END AS present
