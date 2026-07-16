SELECT id, video_id, organization_id, folder_id, shared_by_user_id, sharing_mode,
       shared_at_ms, revoked_at_ms, revision
FROM shared_videos
WHERE video_id = ?1 AND organization_id = ?2 AND revoked_at_ms IS NULL
ORDER BY shared_at_ms, id
LIMIT 101
