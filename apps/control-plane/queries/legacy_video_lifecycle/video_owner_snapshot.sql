SELECT
  video.id AS mapped_video_id,
  alias.legacy_video_id,
  video.owner_id,
  video.organization_id,
  media.object_prefix,
  media.source_type,
  media.transcription_status,
  video.title,
  video.state,
  video.duration_ms,
  video.folder_id,
  video.privacy,
  video.metadata_json,
  video.legacy_public,
  video.legacy_password_hash,
  video.legacy_settings_json,
  video.legacy_metadata_json,
  video.legacy_is_screenshot,
  video.legacy_duration_seconds,
  video.legacy_storage_width,
  video.legacy_storage_height,
  video.legacy_storage_fps,
  owner_alias.legacy_user_id AS legacy_owner_id
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id = alias.mapped_video_id
JOIN legacy_mobile_cap_media_v1 media ON media.mapped_video_id = video.id
JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = video.owner_id
JOIN organizations organization
  ON organization.id = video.organization_id AND organization.status = 'active'
WHERE alias.legacy_video_id = ?2
  AND video.owner_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
LIMIT 2;
