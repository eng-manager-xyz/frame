INSERT INTO shared_videos(
  id, video_id, organization_id, folder_id, shared_by_user_id,
  sharing_mode, shared_at_ms, revoked_at_ms, revision, last_operation_id
)
VALUES (?1, ?2, ?3, NULL, ?4, 'organization', ?5, NULL, 0, ?6)
