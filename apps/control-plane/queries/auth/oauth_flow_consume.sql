UPDATE auth_oauth_flows_v2
SET consumed_at_ms = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE id = ?1
  AND revision = ?2
  AND consumed_at_ms IS NULL
  AND revoked = 0
  AND created_at_ms <= ?3
  AND expires_at_ms > ?3
