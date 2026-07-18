INSERT INTO organizations(
  id, owner_id, name, status, settings_json, created_at_ms, updated_at_ms,
  revision, authority_version, legacy_user_account_name, legacy_icon_key,
  legacy_organization_library_revision, legacy_organization_library_last_operation_id,
  last_operation_id
)
VALUES (
  ?1, ?2,
  CASE WHEN length(?3) BETWEEN 1 AND 160 THEN ?3 ELSE substr(?3, 1, 160) END,
  'active', '{}', ?5, ?5, 1, 1, ?3, ?4, 1, ?6, ?6
)
