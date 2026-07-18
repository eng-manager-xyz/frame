INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM users
         WHERE id = ?2
           AND organization_preference_revision = ?3
           AND default_organization_id IS ?4
           AND active_organization_id IS ?5
           AND organization_last_operation_id = ?6
       ) THEN 1 ELSE 0 END
