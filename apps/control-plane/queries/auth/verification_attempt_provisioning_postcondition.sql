SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_verification_challenges_v2 v
  JOIN auth_identity_provisioning_grants_v2 g
    ON g.id = ?10
   AND g.user_id = v.user_id
   AND g.identity_revision = v.provisioning_revision
   AND g.identifier_key_version = ?3
   AND g.identifier_digest = ?4
   AND g.expires_at_ms = v.expires_at_ms
   AND g.created_at_ms = ?8
   AND g.last_operation_id = ?9
  WHERE v.id = ?1
    AND v.user_id = ?2
    AND v.identifier_key_version = ?3
    AND v.identifier_digest = ?4
    AND v.secret_key_version = ?5
    AND v.secret_digest = ?6
    AND v.purpose = 'identity_provisioning'
    AND v.provisioning_revision = ?7
    AND v.state = 'consumed'
    AND v.consumed_at_ms = ?8
    AND v.last_operation_id = ?9
    AND NOT EXISTS (
      SELECT 1 FROM auth_identities_v2 i WHERE i.user_id = v.user_id
    )
    AND NOT EXISTS (
      SELECT 1 FROM auth_identifier_digests_v2 d
      WHERE d.key_version = ?3 AND d.digest = ?4
    )
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?9
        AND a.action = 'verification_consume'
        AND a.outcome = 'allow'
        AND a.reason = 'verification_completed'
    )
) THEN 1 ELSE 0 END AS present
