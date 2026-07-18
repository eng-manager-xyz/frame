INSERT INTO videos(
  id, owner_id, title, state, source_object_key, playback_object_key,
  duration_ms, created_at_ms, updated_at_ms, organization_id, folder_id,
  privacy, revision, last_operation_id, legacy_public,
  legacy_duration_seconds, legacy_storage_width, legacy_storage_height,
  legacy_storage_fps
) VALUES (
  ?1, ?2, ?3, 'uploading', ?14, NULL,
  ?4, ?5, ?5, ?6, ?7,
  ?8, 0, ?9, ?10,
  ?11, ?12, ?13, NULL
);
