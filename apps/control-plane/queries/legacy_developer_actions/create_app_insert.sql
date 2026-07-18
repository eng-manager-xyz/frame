INSERT INTO legacy_developer_apps_v1(
  id, legacy_app_id, owner_id, name, environment, logo_url, deleted_at_ms,
  created_at_ms, updated_at_ms, revision, authority_version, last_operation_id
)
VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?6, 0, 0, ?7)
