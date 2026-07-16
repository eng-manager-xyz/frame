INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM comments
  WHERE id=?2 AND organization_id=?3 AND video_id=?4 AND deleted_at_ms=?5
    AND updated_at_ms=?5 AND revision=?6 AND last_operation_id=?7
) THEN 1 ELSE 0 END
