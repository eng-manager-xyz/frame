SELECT
  operation_id,
  source_operation_id,
  action,
  actor_id,
  organization_id,
  mapped_video_id,
  legacy_video_id,
  request_key_digest,
  request_digest,
  destination_mapped_video_id,
  destination_legacy_video_id,
  source_prefix,
  destination_prefix,
  result_json,
  state,
  failure_code,
  created_at_ms,
  completed_at_ms
FROM legacy_video_lifecycle_operations_v1
WHERE source_operation_id = ?1
  AND actor_id = ?2
  AND request_key_digest = ?3
LIMIT 2;
