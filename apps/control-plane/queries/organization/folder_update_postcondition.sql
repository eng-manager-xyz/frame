INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM folders
         WHERE id = ?2 AND organization_id = ?3 AND space_id = ?4
           AND name = ?5 AND is_public = ?6 AND settings_json = ?7
           AND revision = ?8 AND tree_revision = ?9
           AND deleted_at_ms IS NULL AND last_operation_id = ?10
       ) THEN 1 ELSE 0 END
