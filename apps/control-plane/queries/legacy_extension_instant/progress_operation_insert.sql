INSERT INTO legacy_extension_instant_operations_v1(
  operation_id, source_operation_id, action, actor_id, organization_id,
  legacy_video_id, mapped_video_id, request_digest, uploaded, total,
  source_updated_at_ms, applied, state, created_at_ms, completed_at_ms
)
SELECT
  ?1, 'cap-v1-82dec55d0fbea3db', 'progress', ?2, ?3,
  ?4, ?5, ?6, ?7, ?8,
  ?9,
  CASE WHEN changes() = 1 THEN 1 ELSE 0 END,
  'complete', ?10, ?10;
