INSERT INTO legacy_extension_instant_assertions_v1(operation_id, assertion_kind, accepted)
SELECT ?1, 'delete_cleanup', CASE WHEN COUNT(*) = 1 THEN 1 ELSE 0 END
FROM legacy_extension_instant_recordings_v1 instant
JOIN legacy_extension_instant_operations_v1 operation
  ON operation.operation_id = ?1
 AND operation.legacy_video_id = instant.legacy_video_id
WHERE instant.legacy_video_id = ?2
  AND instant.mapped_video_id = ?3
  AND instant.actor_id = ?4
  AND instant.lifecycle_state = 'deleting'
  AND instant.storage_cleanup_state = 'pending'
  AND operation.action = 'delete'
  AND operation.state = 'pending_storage';
