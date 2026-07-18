SELECT
  video.id AS mapped_video_id,
  video_alias.legacy_video_id AS legacy_video_id,
  media.object_prefix AS object_prefix
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN legacy_mobile_cap_media_v1 media
  ON media.mapped_video_id = video.id
WHERE video.owner_id = ?1
  AND video_alias.legacy_video_id = ?2
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
LIMIT 2;
