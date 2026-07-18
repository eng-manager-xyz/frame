INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM shared_videos
  WHERE id=?2 AND video_id=?3 AND organization_id=?4 AND folder_id IS ?5
    AND shared_by_user_id=?6 AND sharing_mode=?7 AND shared_at_ms=?8
    AND revoked_at_ms IS ?9 AND revision=?10 AND last_operation_id=?11
) THEN 1 ELSE 0 END
