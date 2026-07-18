SELECT
  alias.legacy_video_id,
  video.id AS mapped_video_id,
  video.owner_id,
  video.legacy_property_revision,
  video.legacy_public,
  organization.legacy_allowed_email_restriction,
  CASE WHEN ?2 IS NOT NULL AND (
    video.owner_id=?2
    OR EXISTS (SELECT 1 FROM organization_members member
      WHERE member.organization_id=video.organization_id
        AND member.user_id=?2 AND member.state='active')
    OR EXISTS (SELECT 1 FROM space_videos placement
      JOIN space_members member ON member.space_id=placement.space_id
      JOIN spaces space ON space.id=placement.space_id
      WHERE placement.video_id=video.id AND member.user_id=?2
        AND space.deleted_at_ms IS NULL)
  ) THEN 1 ELSE 0 END AS explicit_access
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id=alias.mapped_video_id
LEFT JOIN organizations organization ON organization.id=video.organization_id
WHERE alias.legacy_video_id=?1 AND video.deleted_at_ms IS NULL
LIMIT 2;
