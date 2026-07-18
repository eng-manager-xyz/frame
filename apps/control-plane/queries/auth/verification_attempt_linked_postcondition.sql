SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_verification_challenges_v2 v
  JOIN auth_identities_v2 i ON i.user_id = v.user_id
  JOIN users u ON u.id = i.user_id AND u.status = 'active'
  JOIN auth_identifier_digests_v2 d
    ON d.key_version = ?3
   AND d.digest = ?4
   AND d.user_id = i.user_id
   AND d.last_operation_id = ?8
  WHERE v.id = ?1
    AND v.user_id = ?2
    AND v.identifier_key_version = ?3
    AND v.identifier_digest = ?4
    AND v.secret_key_version = ?5
    AND v.secret_digest = ?6
    AND v.purpose = 'account_link'
    AND v.state = 'consumed'
    AND v.consumed_at_ms = ?7
    AND v.last_operation_id = ?8
    AND EXISTS (
      SELECT 1 FROM auth_audit_events_v2 a
      WHERE a.operation_id = ?8
        AND a.action = 'account_link'
        AND a.outcome = 'allow'
        AND a.reason = 'linked'
    )
) THEN 1 ELSE 0 END AS present
