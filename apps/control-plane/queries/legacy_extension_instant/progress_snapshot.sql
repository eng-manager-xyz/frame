SELECT
  instant.mapped_video_id,
  instant.organization_id,
  instant.actor_id,
  instant.storage_integration_id,
  instant.upload_id,
  instant.source_object_key,
  instant.lifecycle_state,
  upload.received_bytes,
  upload.expected_bytes,
  upload.updated_at_ms
FROM legacy_extension_instant_recordings_v1 instant
JOIN legacy_collaboration_video_aliases_v1 alias
  ON alias.legacy_video_id = instant.legacy_video_id
 AND alias.mapped_video_id = instant.mapped_video_id
JOIN videos video ON video.id = instant.mapped_video_id
LEFT JOIN video_uploads upload ON upload.id = instant.upload_id
WHERE instant.legacy_video_id = ?1
  AND video.owner_id = instant.actor_id
LIMIT 2;
