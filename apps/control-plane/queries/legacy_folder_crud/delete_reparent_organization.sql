UPDATE shared_videos
SET folder_id = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE organization_id = ?2
  AND revoked_at_ms IS NULL
  AND folder_id IN (
    SELECT folder_id FROM legacy_folder_crud_delete_targets_v1 WHERE operation_id = ?1
  )
