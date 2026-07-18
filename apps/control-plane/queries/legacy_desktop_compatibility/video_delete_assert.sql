INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1, COUNT(*)
FROM videos
WHERE id = ?2 AND owner_id = ?3 AND organization_id = ?4
  AND state = 'deleted' AND deleted_at_ms = ?5
  AND revision = ?6 AND last_operation_id = ?1;
