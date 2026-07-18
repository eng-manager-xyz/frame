UPDATE comments
SET deleted_at_ms=?4, updated_at_ms=?4, revision=revision+1, last_operation_id=?6
WHERE id=?1 AND organization_id=?2 AND video_id=?3
  AND revision=?5 AND deleted_at_ms IS NULL AND created_at_ms<=?4
