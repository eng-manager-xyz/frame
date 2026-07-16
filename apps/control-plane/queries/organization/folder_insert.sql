INSERT INTO folders(
  id, organization_id, space_id, parent_id, created_by_user_id, name,
  is_public, settings_json, created_at_ms, updated_at_ms, deleted_at_ms,
  revision, depth, tree_revision, last_operation_id
) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, NULL, 0,
         COALESCE(parent.depth + 1, 0), ?10, ?11
  FROM (SELECT 1) seed
  LEFT JOIN folders parent
    ON parent.id = ?4 AND parent.organization_id = ?2 AND parent.space_id = ?3
   AND parent.deleted_at_ms IS NULL AND parent.revision = ?12
 WHERE (?4 IS NULL OR parent.id IS NOT NULL)
   AND COALESCE(parent.depth + 1, 0) <= 32
   AND EXISTS (
     SELECT 1 FROM spaces
     WHERE id = ?3 AND organization_id = ?2 AND deleted_at_ms IS NULL AND revision = ?13
   )
