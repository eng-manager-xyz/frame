UPDATE videos
SET folder_id = NULL,
    revision = revision + 1,
    last_operation_id = ?3,
    updated_at_ms = ?4
WHERE id = ?1
  AND organization_id = ?2
  AND deleted_at_ms IS NULL
  AND revision = ?5
  AND folder_id = ?6
  AND EXISTS (
    SELECT 1
    FROM folders folder
    LEFT JOIN spaces folder_space
      ON folder_space.id = folder.space_id
     AND folder_space.organization_id = folder.organization_id
     AND folder_space.deleted_at_ms IS NULL
    WHERE folder.id = videos.folder_id
      AND folder.organization_id = ?2
      AND folder.deleted_at_ms IS NULL
      AND (folder.space_id IS NULL OR folder_space.id IS NOT NULL)
  )
