INSERT INTO legacy_mobile_upload_records_v1(
  mapped_video_id, legacy_video_id, actor_id, legacy_actor_id,
  organization_id, storage_integration_id, upload_id, folder_id,
  raw_file_key, file_name, content_type, declared_content_length,
  duration_seconds, width, height, ignored_fps, lifecycle_state,
  created_at_ms, updated_at_ms, last_operation_id
) VALUES (
  ?1, ?2, ?3, ?4,
  ?5, ?6, ?7, ?8,
  ?9, ?10, ?11, ?12,
  ?13, ?14, ?15, ?16, 'uploading',
  ?17, ?17, ?18
);
