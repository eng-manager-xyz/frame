SELECT
  video.id AS mapped_video_id,
  video.organization_id AS organization_id,
  video_alias.legacy_video_id AS legacy_video_id,
  actor_alias.legacy_user_id AS legacy_actor_id,
  upload.id AS upload_id,
  upload.received_bytes AS received_bytes,
  upload.expected_bytes AS expected_bytes,
  upload.updated_at_ms AS upload_updated_at_ms
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN legacy_collaboration_user_aliases_v1 actor_alias
  ON actor_alias.mapped_user_id = video.owner_id
LEFT JOIN video_uploads upload
  ON upload.id = (
    SELECT candidate.id FROM video_uploads candidate
    WHERE candidate.video_id = video.id
    ORDER BY candidate.updated_at_ms DESC, candidate.id
    LIMIT 1
  )
WHERE video.owner_id = ?1
  AND video_alias.legacy_video_id = ?2
  AND video.organization_id IS NOT NULL
  AND video.deleted_at_ms IS NULL
LIMIT 2;
