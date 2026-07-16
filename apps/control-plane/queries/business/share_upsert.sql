INSERT INTO shared_videos(
  id, video_id, organization_id, folder_id, shared_by_user_id, sharing_mode,
  shared_at_ms, revoked_at_ms, revision, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?11)
ON CONFLICT(id) DO UPDATE SET
  sharing_mode = excluded.sharing_mode,
  revoked_at_ms = excluded.revoked_at_ms,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE shared_videos.organization_id = excluded.organization_id
  AND shared_videos.video_id = excluded.video_id
  AND shared_videos.folder_id IS excluded.folder_id
  AND shared_videos.shared_by_user_id = excluded.shared_by_user_id
  AND shared_videos.shared_at_ms = excluded.shared_at_ms
  AND shared_videos.revision = ?10
