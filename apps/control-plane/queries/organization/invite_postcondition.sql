INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organization_invites
         WHERE id = ?2 AND organization_id = ?3 AND status = ?4
           AND revision = ?5 AND last_operation_id = ?6
       ) THEN 1 ELSE 0 END
