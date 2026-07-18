INSERT INTO legacy_developer_multipart_sessions_v1(
  provider_upload_id, app_id, video_id, object_key, content_type, state,
  initiated_operation_id, created_at_ms, updated_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, 'open', ?6, ?7, ?7)
