INSERT INTO organizations(
  id, owner_id, name, status, settings_json,
  created_at_ms, updated_at_ms, revision
) VALUES(?1, ?2, 'My Organization', 'active', '{}', ?3, ?3, 0)
