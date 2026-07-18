DELETE FROM folders
WHERE id = ?1
  AND organization_id = ?2
  AND deleted_at_ms IS NULL
  AND revision = ?3
  AND tree_revision = ?4
