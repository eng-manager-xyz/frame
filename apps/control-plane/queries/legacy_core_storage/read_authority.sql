SELECT
  media.mapped_video_id,
  media.legacy_video_id,
  media.owner_id,
  media.organization_id,
  media.object_prefix,
  media.source_type,
  upload.raw_file_key,
  integration.id AS storage_integration_id,
  video.privacy,
  video.legacy_public,
  video.legacy_password_hash,
  COALESCE(json_extract(integration.capabilities_json, '$.single_put'), 0) AS supports_single_put,
  COALESCE(json_extract(integration.capabilities_json, '$.multipart'), 0) AS supports_multipart
FROM legacy_mobile_cap_media_v1 media
JOIN videos video
  ON video.id = media.mapped_video_id
 AND video.owner_id = media.owner_id
 AND video.organization_id = media.organization_id
JOIN organizations organization
  ON organization.id = media.organization_id
 AND organization.status = 'active'
JOIN storage_integrations integration
  ON integration.organization_id = media.organization_id
 AND integration.provider = 'r2'
 AND integration.state = 'active'
LEFT JOIN legacy_mobile_cap_uploads_v1 upload
  ON upload.mapped_video_id = media.mapped_video_id
WHERE media.legacy_video_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
  AND json_extract(integration.capabilities_json, '$.single_put') = 1
  AND (
    (video.legacy_public = 1 AND video.legacy_password_hash IS NULL)
    OR video.owner_id = ?2
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = media.organization_id
        AND member.user_id = ?2 AND member.state = 'active'
    )
    OR ?3 = 1
  )
ORDER BY integration.updated_at_ms DESC, integration.id
LIMIT 2;
