INSERT INTO developer_apps(
  id, owner_user_id, organization_id, name, environment, status,
  created_at_ms, updated_at_ms, deleted_at_ms, revision,
  authority_version, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
ON CONFLICT(id) DO UPDATE SET
  name = excluded.name,
  status = excluded.status,
  updated_at_ms = excluded.updated_at_ms,
  deleted_at_ms = excluded.deleted_at_ms,
  revision = excluded.revision,
  authority_version = excluded.authority_version,
  last_operation_id = excluded.last_operation_id
WHERE developer_apps.organization_id = excluded.organization_id
  AND developer_apps.owner_user_id = excluded.owner_user_id
  AND developer_apps.environment = excluded.environment
  AND developer_apps.created_at_ms = excluded.created_at_ms
  AND developer_apps.revision = ?13
  AND developer_apps.authority_version = ?14
