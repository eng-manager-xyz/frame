UPDATE videos
SET title = CASE WHEN ?2 = 1 THEN ?3 ELSE title END,
    legacy_metadata_json = CASE WHEN ?4 = 1 THEN ?5 ELSE legacy_metadata_json END,
    legacy_public = CASE WHEN ?6 = 1 THEN ?7 ELSE legacy_public END,
    legacy_password_hash = CASE WHEN ?8 = 1 THEN ?9 ELSE legacy_password_hash END,
    legacy_settings_json = CASE WHEN ?10 = 1 THEN ?11 ELSE legacy_settings_json END,
    updated_at_ms = ?12,
    revision = revision + 1,
    legacy_property_revision = legacy_property_revision + 1,
    legacy_property_last_operation_id = ?13
WHERE id = ?1 AND revision = ?14 AND legacy_property_revision = ?15;
