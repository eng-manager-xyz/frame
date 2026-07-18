INSERT INTO legacy_developer_videos_v1(
  id, legacy_video_id, app_id, external_user_id, name, s3_key, metadata_json,
  created_at_ms, updated_at_ms, revision, last_operation_id
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, ?9)
