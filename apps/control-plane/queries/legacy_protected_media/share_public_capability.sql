SELECT
  'public_video_capability' AS capability_kind,
  0 AS capability_ordinal,
  video.id AS capability_subject_id,
  video.legacy_property_revision AS capability_revision,
  NULL AS password_hash
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id = alias.mapped_video_id
LEFT JOIN organizations organization ON organization.id = video.organization_id
WHERE alias.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.legacy_public = 1
  AND video.legacy_password_hash IS NULL
  AND COALESCE(TRIM(organization.legacy_allowed_email_restriction),'') = ''
  AND NOT EXISTS (
    SELECT 1
    FROM space_videos placement
    JOIN spaces space ON space.id = placement.space_id
    WHERE placement.video_id = video.id
      AND space.deleted_at_ms IS NULL
      AND space.legacy_password_hash IS NOT NULL
  )
LIMIT 2;
