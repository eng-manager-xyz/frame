SELECT
  media.mapped_video_id,
  media.legacy_video_id,
  media.owner_id,
  media.organization_id,
  media.object_prefix,
  media.source_type,
  video.title,
  video.legacy_is_screenshot,
  integration.id AS storage_integration_id,
  upload.phase,
  upload.processing_progress,
  upload.updated_at_ms AS upload_updated_at_ms,
  upload.raw_file_key,
  edit_source.source_key AS edit_source_key
FROM legacy_mobile_cap_media_v1 media
JOIN videos video ON video.id = media.mapped_video_id AND video.owner_id = ?1
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
WHERE media.legacy_video_id = ?2
  AND media.owner_id = ?1
  AND video.deleted_at_ms IS NULL AND video.state <> 'deleted'
ORDER BY integration.updated_at_ms DESC, integration.id
LIMIT 2;
