INSERT INTO videos(
  id, owner_id, title, state, duration_ms, created_at_ms, updated_at_ms,
  organization_id, folder_id, privacy, metadata_json, revision,
  metadata_schema_version, metadata_checksum, comments_enabled,
  last_operation_id, legacy_public, legacy_password_hash,
  legacy_settings_json, legacy_metadata_json, legacy_property_revision,
  legacy_property_last_operation_id, legacy_is_screenshot,
  legacy_duration_seconds, legacy_storage_width, legacy_storage_height,
  legacy_storage_fps
)
SELECT
  ?2, source.owner_id, source.title, source.state, source.duration_ms, ?3, ?3,
  source.organization_id, source.folder_id, source.privacy, source.metadata_json, 0,
  source.metadata_schema_version, source.metadata_checksum, source.comments_enabled,
  ?4, source.legacy_public, NULL,
  source.legacy_settings_json, source.legacy_metadata_json, 0,
  NULL, source.legacy_is_screenshot,
  source.legacy_duration_seconds, source.legacy_storage_width,
  source.legacy_storage_height, source.legacy_storage_fps
FROM videos source
WHERE source.id = ?1
  AND source.owner_id = ?5
  AND source.deleted_at_ms IS NULL
  AND source.state <> 'deleted';
