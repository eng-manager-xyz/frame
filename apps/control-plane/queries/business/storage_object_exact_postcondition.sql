INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM storage_objects
  WHERE id=?2 AND organization_id=?3 AND integration_id=?4 AND video_id IS ?5
    AND object_key=?6 AND role=?7 AND object_version=?8 AND state=?9
    AND bytes=?10 AND content_type=?11 AND checksum_sha256=?12
    AND created_at_ms=?13 AND deleted_at_ms IS ?14 AND updated_at_ms=?15
    AND revision=?16 AND last_operation_id=?17
) THEN 1 ELSE 0 END
