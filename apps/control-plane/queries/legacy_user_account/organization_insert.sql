INSERT INTO organizations(
  id, owner_id, name, legacy_user_account_name, status, settings_json,
  created_at_ms, updated_at_ms, revision, authority_version, last_operation_id
) VALUES (?1, ?2, ?3, ?4, 'active', '{}', ?5, ?5, 0, 0, ?6);
