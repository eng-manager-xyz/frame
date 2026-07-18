INSERT INTO legacy_mobile_cap_delete_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, ?2, 1, COUNT(*)
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 alias
  ON alias.mapped_video_id = video.id
JOIN legacy_mobile_cap_delete_operations_v1 operation
  ON operation.mapped_video_id = video.id
 AND operation.operation_id = ?1
WHERE video.id = ?3
  AND video.owner_id = ?4
  AND alias.legacy_video_id = ?5
  AND video.state = 'deleted'
  AND video.deleted_at_ms = ?6
  AND operation.state = 'storage_pending';
