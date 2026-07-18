INSERT INTO legacy_extension_instant_recordings_v1(
  legacy_video_id, mapped_video_id, upload_id, organization_id, actor_id,
  storage_integration_id, storage_prefix, source_object_key,
  supports_upload_progress, lifecycle_state, storage_cleanup_state,
  created_at_ms, last_operation_id
) VALUES (
  ?1, ?2, CASE WHEN ?9 = 1 THEN ?3 ELSE NULL END, ?4, ?5,
  ?6, ?7, ?8,
  ?9, 'active', 'not_requested', ?10, ?11
);
