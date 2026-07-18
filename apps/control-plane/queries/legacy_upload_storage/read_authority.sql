SELECT
  media.mapped_video_id,
  media.legacy_video_id,
  media.owner_id,
  media.organization_id,
  media.object_prefix,
  media.source_type,
  video.title,
  video.legacy_is_screenshot,
  video.legacy_public,
  video.legacy_password_hash,
  (SELECT actor.email FROM users actor
   WHERE actor.id = ?1 AND actor.status = 'active' AND actor.deleted_at_ms IS NULL
   LIMIT 1) AS actor_email,
  organization.legacy_allowed_email_restriction AS allowed_email_restriction,
  integration.id AS storage_integration_id,
  upload.uploaded,
  upload.total,
  upload.started_at_ms,
  upload.updated_at_ms AS upload_updated_at_ms,
  upload.phase,
  upload.processing_progress,
  upload.processing_message,
  upload.processing_error,
  upload.raw_file_key,
  edit_source.source_key AS edit_source_key,
  CASE WHEN
    media.owner_id = ?1
    OR EXISTS (
      SELECT 1 FROM legacy_upload_storage_organization_shares_v1 shared
      JOIN organization_members member
        ON member.organization_id = shared.organization_id
       AND member.user_id = ?1 AND member.state = 'active'
      WHERE shared.mapped_video_id = media.mapped_video_id
    )
    OR EXISTS (
      SELECT 1 FROM legacy_upload_storage_space_shares_v1 placement
      JOIN spaces space ON space.id = placement.space_id AND space.deleted_at_ms IS NULL
      JOIN space_members member
        ON member.space_id = placement.space_id AND member.user_id = ?1
      WHERE placement.mapped_video_id = media.mapped_video_id
    )
  THEN 1 ELSE 0 END AS explicit_view
  ,CASE WHEN
    media.owner_id = ?1
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = media.organization_id
        AND member.user_id = ?1 AND member.state = 'active'
    )
    OR EXISTS (
      SELECT 1 FROM legacy_upload_storage_organization_shares_v1 shared
      JOIN organization_members member
        ON member.organization_id = shared.organization_id
       AND member.user_id = ?1 AND member.state = 'active'
      WHERE shared.mapped_video_id = media.mapped_video_id
    )
    OR EXISTS (
      SELECT 1 FROM legacy_upload_storage_space_shares_v1 placement
      JOIN space_members member
        ON member.space_id = placement.space_id AND member.user_id = ?1
      WHERE placement.mapped_video_id = media.mapped_video_id
    )
  THEN 1 ELSE 0 END AS can_download
FROM legacy_mobile_cap_media_v1 media
JOIN videos video ON video.id = media.mapped_video_id
JOIN organizations organization
  ON organization.id = media.organization_id AND organization.status = 'active'
JOIN storage_integrations integration
  ON integration.organization_id = media.organization_id
 AND integration.provider = 'r2' AND integration.state = 'active'
 AND json_extract(integration.capabilities_json, '$.single_put') = 1
LEFT JOIN legacy_mobile_cap_uploads_v1 upload
  ON upload.mapped_video_id = media.mapped_video_id
LEFT JOIN legacy_upload_storage_edit_sources_v1 edit_source
  ON edit_source.mapped_video_id = media.mapped_video_id
 AND EXISTS (SELECT 1 FROM video_edits edit WHERE edit.video_id = media.mapped_video_id)
WHERE media.legacy_video_id = ?2
  AND video.deleted_at_ms IS NULL AND video.state <> 'deleted'
ORDER BY integration.updated_at_ms DESC, integration.id
LIMIT 2;
