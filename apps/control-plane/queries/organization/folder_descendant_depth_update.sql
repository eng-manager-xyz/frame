UPDATE folders
SET depth = depth
      + COALESCE((SELECT depth + 1 FROM folders WHERE id = ?4), 0)
      - (SELECT depth FROM folders WHERE id = ?3),
    tree_revision = tree_revision + 1,
    updated_at_ms = ?5,
    last_operation_id = ?6
WHERE organization_id = ?1 AND space_id = ?2 AND id <> ?3
  AND id IN (
    SELECT descendant_id FROM organization_folder_closure_v1
    WHERE organization_id = ?1 AND space_id = ?2 AND ancestor_id = ?3
  )
