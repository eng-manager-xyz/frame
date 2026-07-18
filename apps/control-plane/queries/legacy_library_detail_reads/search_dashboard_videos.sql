SELECT
  video_alias.legacy_video_id AS legacy_video_id,
  video.title AS video_name,
  owner.display_name AS owner_name,
  video.created_at_ms AS created_at_ms,
  video.legacy_duration_seconds AS duration_seconds,
  video.legacy_is_screenshot AS is_screenshot,
  video.legacy_effective_created_at_ms AS effective_created_at_ms
FROM videos video
JOIN organizations organization
  ON organization.id = video.organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
LEFT JOIN users owner
  ON owner.id = video.owner_id
WHERE video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
  AND video.title LIKE ?3 ESCAPE '!'
  AND (
    video.owner_id = ?1
    OR EXISTS (
      SELECT 1
      FROM shared_videos shared
      WHERE shared.video_id = video.id
        AND shared.organization_id = ?2
        AND shared.revoked_at_ms IS NULL
    )
    OR EXISTS (
      SELECT 1
      FROM space_videos placement
      JOIN spaces space
        ON space.id = placement.space_id
       AND space.organization_id = ?2
       AND space.deleted_at_ms IS NULL
      LEFT JOIN space_members membership
        ON membership.space_id = space.id
       AND membership.user_id = ?1
       AND membership.state = 'active'
      WHERE placement.video_id = video.id
        AND (
          space.created_by_user_id = ?1
          OR space.is_public = 1
          OR membership.user_id IS NOT NULL
        )
    )
  )
ORDER BY
  CASE WHEN video.title LIKE ?4 ESCAPE '!' THEN 0 ELSE 1 END,
  video.legacy_effective_created_at_us DESC
LIMIT 8;
