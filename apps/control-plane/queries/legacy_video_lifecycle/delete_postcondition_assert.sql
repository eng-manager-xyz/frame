INSERT INTO legacy_video_lifecycle_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'postcondition', 1, COUNT(*)
FROM videos
WHERE id = ?2
  AND owner_id = ?3
  AND state = 'deleted'
  AND deleted_at_ms IS NOT NULL
  AND last_operation_id = ?1;
