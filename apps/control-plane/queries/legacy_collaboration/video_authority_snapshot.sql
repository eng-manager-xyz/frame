SELECT
  video_alias.mapped_video_id,
  video.owner_id,
  video.revision AS video_revision,
  COALESCE((
    SELECT MAX(shared.revision)
    FROM shared_videos shared
    WHERE shared.video_id = video.id
      AND shared.organization_id = ?3
      AND shared.revoked_at_ms IS NULL
  ), -1) AS shared_revision,
  CASE WHEN video.owner_id = ?2 THEN 'owner' ELSE 'active_organization_share' END AS authority_kind
FROM legacy_collaboration_video_aliases_v1 video_alias
JOIN videos video ON video.id = video_alias.mapped_video_id
WHERE video_alias.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL
  AND (
    video.owner_id = ?2
    OR EXISTS (
      SELECT 1 FROM shared_videos shared
      WHERE shared.video_id = video.id
        AND shared.organization_id = ?3
        AND shared.revoked_at_ms IS NULL
    )
  )
LIMIT 2;
