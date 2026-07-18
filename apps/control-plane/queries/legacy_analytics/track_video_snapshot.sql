SELECT
  video.id AS native_video_id,
  video.owner_id,
  video.organization_id,
  video.title AS video_name,
  video.created_at_ms,
  video.updated_at_ms,
  CASE WHEN EXISTS (
    SELECT 1 FROM video_uploads upload WHERE upload.video_id = video.id
  ) THEN 1 ELSE 0 END AS has_active_upload,
  CASE WHEN video.legacy_analytics_first_view_email_sent_at_ms IS NOT NULL OR EXISTS (
    SELECT 1 FROM legacy_analytics_email_outbox_v1 email
    WHERE email.video_id = video.id
  ) THEN 1 ELSE 0 END AS first_view_email_claimed,
  owner.email AS owner_email,
  COALESCE(NULLIF(owner.display_name, ''), NULLIF(owner.email, ''), 'Someone') AS owner_name,
  CASE WHEN EXISTS (
    SELECT 1 FROM organizations active_organization
    WHERE active_organization.id = owner.active_organization_id
      AND active_organization.status = 'active'
  ) THEN owner.active_organization_id ELSE NULL END AS owner_active_organization_id,
  COALESCE(json_extract(owner.preferences_json, '$.notifications.pauseViews'), 0)
    AS pause_views,
  COALESCE(json_extract(owner.preferences_json, '$.notifications.pauseAnonViews'), 0)
    AS pause_anonymous_views
FROM videos video
JOIN users owner ON owner.id = video.owner_id
WHERE video.deleted_at_ms IS NULL AND (
  video.id = ?1 OR EXISTS (
    SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
    WHERE alias.mapped_video_id = video.id AND alias.legacy_video_id = ?1
  )
)
ORDER BY CASE WHEN video.id = ?1 THEN 0 ELSE 1 END
LIMIT 1
