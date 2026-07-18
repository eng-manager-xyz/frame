DELETE FROM organization_folder_closure_v1
WHERE organization_id = ?1 AND space_id = ?2
  AND descendant_id IN (
    SELECT descendant_id FROM organization_folder_closure_v1
    WHERE organization_id = ?1 AND space_id = ?2 AND ancestor_id = ?3
  )
  AND ancestor_id IN (
    SELECT ancestor_id FROM organization_folder_closure_v1
    WHERE organization_id = ?1 AND space_id = ?2 AND descendant_id = ?3 AND ancestor_id <> ?3
  )
