INSERT INTO spaces(
  id, organization_id, created_by_user_id, name, is_primary, is_public,
  settings_json, created_at_ms, updated_at_ms, deleted_at_ms, revision,
  authority_version, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL, 0, 0, ?9)
