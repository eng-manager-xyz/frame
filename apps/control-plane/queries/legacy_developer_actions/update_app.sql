UPDATE legacy_developer_apps_v1
SET
  name = CASE WHEN ?6 = 1 THEN ?7 ELSE name END,
  environment = CASE WHEN ?8 = 1 THEN ?9 ELSE environment END,
  logo_url = CASE WHEN ?10 = 1 THEN ?11 ELSE logo_url END,
  updated_at_ms = ?12,
  revision = revision + 1,
  last_operation_id = ?1
WHERE id = ?2 AND owner_id = ?3 AND deleted_at_ms IS NULL
  AND revision = ?4 AND authority_version = ?5
  AND ?13 = 1
