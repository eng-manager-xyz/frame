INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM video_uploads
  WHERE id=?2 AND organization_id=?3 AND video_id=?4 AND expected_bytes=?5
    AND idempotency_key=?6 AND source_object_key=?7 AND source_version=?8
    AND content_type=?9 AND created_at_ms=?10
) THEN 1 ELSE 0 END
