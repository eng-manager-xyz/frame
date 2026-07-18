SELECT
  video.id AS mapped_video_id,
  video_alias.legacy_video_id AS legacy_video_id,
  video.owner_id AS owner_id,
  video.organization_id AS organization_id,
  COALESCE(
    media.object_prefix,
    owner_alias.legacy_user_id || '/' || video_alias.legacy_video_id || '/'
  ) AS object_prefix,
  media.transcription_status AS transcription_status,
  video.legacy_public AS legacy_public,
  video.legacy_password_hash AS video_password_hash,
  organization.legacy_allowed_email_restriction AS allowed_email_restriction
FROM legacy_collaboration_video_aliases_v1 video_alias
JOIN videos video ON video.id = video_alias.mapped_video_id
JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = video.owner_id
LEFT JOIN organizations organization ON organization.id = video.organization_id
LEFT JOIN legacy_mobile_cap_media_v1 media ON media.mapped_video_id = video.id
WHERE video_alias.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL
LIMIT 2;
