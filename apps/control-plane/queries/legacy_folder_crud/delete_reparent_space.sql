UPDATE space_videos
SET folder_id = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE space_id = ?3
  AND folder_id IN (
    SELECT folder_id FROM legacy_folder_crud_delete_targets_v1 WHERE operation_id = ?1
  )
  AND EXISTS (
    SELECT 1 FROM spaces s
    WHERE s.id = ?3 AND s.organization_id = ?2 AND s.deleted_at_ms IS NULL
  )
