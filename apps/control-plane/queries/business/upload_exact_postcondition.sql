INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM video_uploads
  WHERE id=?2 AND organization_id=?3 AND state=?4 AND received_bytes=?5
    AND checksum_sha256 IS ?6 AND event_sequence=?7 AND event_fingerprint=?8
    AND updated_at_ms=?9 AND revision=?10 AND last_operation_id=?11
) THEN 1 ELSE 0 END
