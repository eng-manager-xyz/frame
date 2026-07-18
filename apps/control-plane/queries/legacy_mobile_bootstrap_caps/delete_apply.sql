UPDATE videos
SET state = 'deleted',
    deleted_at_ms = ?4,
    updated_at_ms = CASE WHEN updated_at_ms < ?4 THEN ?4 ELSE updated_at_ms END,
    revision = revision + 1
WHERE id = ?3
  AND owner_id = ?1
  AND deleted_at_ms IS NULL
  AND state <> 'deleted'
  AND EXISTS (
    SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
    WHERE alias.mapped_video_id = videos.id
      AND alias.legacy_video_id = ?2
  );
