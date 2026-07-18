INSERT INTO organizations(
  id, owner_id, name, status, settings_json, created_at_ms, updated_at_ms,
  tombstoned_at_ms, revision, authority_version, retention_until_ms,
  recovered_at_ms, last_operation_id
) VALUES (?1, ?2, ?3, 'active', ?4, ?5, ?5, NULL, 0, 0, NULL, NULL, ?6)
