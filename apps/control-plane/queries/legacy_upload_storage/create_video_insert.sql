INSERT INTO videos(
  id, owner_id, title, state, source_object_key, playback_object_key,
  duration_ms, created_at_ms, updated_at_ms, organization_id, folder_id,
  privacy, revision, last_operation_id, legacy_public, legacy_duration_seconds,
  legacy_is_screenshot
) VALUES (
  ?1, ?2, ?3, 'uploading', ?4, NULL,
  ?5, ?6, ?6, ?7, ?8,
  ?9, 0, ?10, ?11, ?12, ?13
);
