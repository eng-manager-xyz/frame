INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM spaces
         WHERE id = ?2 AND organization_id = ?3
           AND name = ?4 AND is_public = ?5 AND settings_json = ?6
           AND revision = ?7 AND deleted_at_ms IS NULL
           AND last_operation_id = ?8
       ) THEN 1 ELSE 0 END
