SELECT
  actor.id AS actor_id,
  actor.active_organization_id,
  organization.id AS organization_id,
  CASE WHEN organization.owner_id = actor.id OR EXISTS (
    SELECT 1 FROM organization_members member
    WHERE member.organization_id = organization.id
      AND member.user_id = actor.id AND member.state = 'active'
  ) THEN 1 ELSE 0 END AS organization_allowed,
  CASE WHEN ?3 IS NULL OR EXISTS (
    SELECT 1 FROM spaces space
    WHERE space.id = ?3 AND space.organization_id = organization.id
      AND space.deleted_at_ms IS NULL
  ) THEN 1 ELSE 0 END AS space_allowed,
  CASE WHEN ?4 IS NULL OR EXISTS (
    SELECT 1 FROM videos video
    WHERE video.deleted_at_ms IS NULL AND video.organization_id = organization.id
      AND (video.id = ?4 OR EXISTS (
        SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
        WHERE alias.mapped_video_id = video.id AND alias.legacy_video_id = ?4
      ))
  ) THEN 1 ELSE 0 END AS video_allowed,
  (
    SELECT MIN(video.created_at_ms) FROM videos video
    WHERE video.organization_id = organization.id AND video.deleted_at_ms IS NULL
      AND (?3 IS NULL OR EXISTS (
        SELECT 1 FROM space_videos placement
        WHERE placement.space_id = ?3 AND placement.video_id = video.id
      ))
      AND (?4 IS NULL OR video.id = ?4 OR EXISTS (
        SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
        WHERE alias.mapped_video_id = video.id AND alias.legacy_video_id = ?4
      ))
  ) AS lifetime_start_ms
FROM users actor
JOIN organizations organization ON organization.id = ?2 AND organization.status = 'active'
WHERE actor.id = ?1 AND actor.deleted_at_ms IS NULL
LIMIT 1
