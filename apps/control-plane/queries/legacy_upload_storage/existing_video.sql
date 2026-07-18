SELECT alias.mapped_video_id, alias.legacy_video_id, video.owner_id
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id = alias.mapped_video_id
WHERE alias.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL AND video.state <> 'deleted'
LIMIT 2;
