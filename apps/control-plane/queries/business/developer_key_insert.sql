INSERT INTO developer_api_keys(
  id, app_id, key_digest, key_type, name, created_at_ms, last_used_at_ms,
  revoked_at_ms, display_prefix, revision, last_operation_id
)
SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11
WHERE EXISTS (
  SELECT 1 FROM developer_apps app
  WHERE app.id = ?2 AND app.organization_id = ?12 AND app.status = 'active'
)
