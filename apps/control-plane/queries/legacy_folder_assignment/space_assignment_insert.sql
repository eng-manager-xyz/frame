INSERT INTO space_videos(
  space_id, video_id, folder_id, added_by_user_id, added_at_ms,
  revision, last_operation_id
)
VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)
