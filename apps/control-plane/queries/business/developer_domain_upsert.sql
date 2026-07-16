INSERT INTO developer_app_domains(
  app_id, domain_ascii, created_at_ms, verified_at_ms, revision,
  last_operation_id
)
SELECT ?1,?2,?3,?4,?5,?7
WHERE EXISTS (
  SELECT 1 FROM developer_apps app
  WHERE app.id = ?1 AND app.organization_id = ?8 AND app.status <> 'deleted'
)
ON CONFLICT(app_id, domain_ascii) DO UPDATE SET
  verified_at_ms = excluded.verified_at_ms,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE developer_app_domains.created_at_ms = excluded.created_at_ms
  AND developer_app_domains.revision = ?6
