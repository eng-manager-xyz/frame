SELECT
  instant.mapped_video_id,
  instant.organization_id,
  instant.actor_id,
  instant.upload_id,
  instant.storage_prefix,
  instant.lifecycle_state,
  (
    SELECT operation.operation_id
    FROM legacy_extension_instant_operations_v1 operation
    WHERE operation.legacy_video_id = instant.legacy_video_id
      AND operation.action = 'delete'
      AND operation.state = 'pending_storage'
    ORDER BY operation.created_at_ms, operation.operation_id
    LIMIT 1
  ) AS pending_operation_id
FROM legacy_extension_instant_recordings_v1 instant
JOIN legacy_collaboration_video_aliases_v1 alias
  ON alias.legacy_video_id = instant.legacy_video_id
 AND alias.mapped_video_id = instant.mapped_video_id
JOIN videos video ON video.id = instant.mapped_video_id
WHERE instant.legacy_video_id = ?1
  AND video.owner_id = instant.actor_id
  AND instant.lifecycle_state IN ('active', 'deleting')
LIMIT 2;
