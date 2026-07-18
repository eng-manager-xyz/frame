INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM imported_videos
  WHERE id=?2 AND organization_id=?3 AND video_id IS ?4 AND provider=?5
    AND external_id_digest=?6 AND idempotency_key=?7 AND created_at_ms=?8
) THEN 1 ELSE 0 END
