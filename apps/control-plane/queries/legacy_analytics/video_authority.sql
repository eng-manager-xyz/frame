SELECT
  video.id AS native_video_id,
  video.owner_id,
  video.organization_id,
  video.legacy_public,
  CASE WHEN video.owner_id = ?2 THEN 1 ELSE 0 END AS actor_is_owner,
  CASE WHEN EXISTS (
    SELECT 1 FROM shared_videos shared
    JOIN organization_members member
      ON member.organization_id = shared.organization_id
     AND member.user_id = ?2 AND member.state = 'active'
    WHERE shared.video_id = video.id AND shared.revoked_at_ms IS NULL
  ) THEN 1 ELSE 0 END AS actor_has_organization_share,
  CASE WHEN EXISTS (
    SELECT 1 FROM space_videos placement
    JOIN space_members member
      ON member.space_id = placement.space_id AND member.user_id = ?2
     AND member.state = 'active'
    JOIN spaces space ON space.id = placement.space_id AND space.deleted_at_ms IS NULL
    WHERE placement.video_id = video.id
  ) THEN 1 ELSE 0 END AS actor_has_space_share,
  CASE WHEN video.legacy_password_hash IS NOT NULL OR EXISTS (
    SELECT 1 FROM space_videos placement
    JOIN spaces space ON space.id = placement.space_id
    WHERE placement.video_id = video.id
      AND space.deleted_at_ms IS NULL AND space.legacy_password_hash IS NOT NULL
  ) THEN 1 ELSE 0 END AS password_required,
  CASE WHEN EXISTS (
    SELECT 1 FROM legacy_analytics_password_grants_v1 grant_row
    WHERE grant_row.grant_digest = ?3 AND grant_row.video_id = video.id
      AND grant_row.expires_at_ms > ?4
  ) THEN 1 ELSE 0 END AS password_granted,
  CASE WHEN NOT EXISTS (
    SELECT 1 FROM organization_allowed_domains domain
    WHERE domain.organization_id = video.organization_id
      AND domain.verified_at_ms IS NOT NULL
  ) OR EXISTS (
    SELECT 1 FROM users actor
    JOIN organization_allowed_domains domain
      ON domain.organization_id = video.organization_id
     AND domain.verified_at_ms IS NOT NULL
    WHERE actor.id = ?2
      AND lower(actor.email) LIKE '%@' || lower(domain.domain_ascii)
  ) THEN 1 ELSE 0 END AS email_allowed
FROM videos video
WHERE video.deleted_at_ms IS NULL AND (
  video.id = ?1 OR EXISTS (
    SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
    WHERE alias.mapped_video_id = video.id AND alias.legacy_video_id = ?1
  )
)
ORDER BY CASE WHEN video.id = ?1 THEN 0 ELSE 1 END
LIMIT 1
