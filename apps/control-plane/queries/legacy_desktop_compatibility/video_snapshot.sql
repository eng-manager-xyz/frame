SELECT
  video.id AS video_id,
  video.organization_id,
  video.owner_id,
  video.revision,
  video_alias.legacy_video_id
FROM videos video
LEFT JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
WHERE video.owner_id = ?1
  AND video.state <> 'deleted'
  AND video.deleted_at_ms IS NULL
  AND (video.id = ?2 OR video_alias.legacy_video_id = ?3)
ORDER BY CASE WHEN video_alias.legacy_video_id = ?3 THEN 0 ELSE 1 END
LIMIT 2;
