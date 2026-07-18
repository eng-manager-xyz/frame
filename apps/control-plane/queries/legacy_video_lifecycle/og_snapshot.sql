SELECT
  video.id AS mapped_video_id,
  alias.legacy_video_id,
  video.owner_id,
  media.object_prefix,
  video.legacy_public,
  video.privacy
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id = alias.mapped_video_id
JOIN legacy_mobile_cap_media_v1 media ON media.mapped_video_id = video.id
WHERE alias.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
LIMIT 2;
