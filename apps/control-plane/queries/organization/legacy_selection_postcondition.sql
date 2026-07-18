INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM users
         WHERE id = ?2
           AND active_organization_id = ?3
           AND organization_last_operation_id = ?4
       ) THEN 1 ELSE 0 END
