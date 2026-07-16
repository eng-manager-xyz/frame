INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM space_members sm
         JOIN spaces s ON s.id = sm.space_id
         JOIN organization_members m
           ON m.organization_id = s.organization_id AND m.user_id = sm.user_id
         WHERE sm.space_id = ?2 AND sm.user_id = ?3 AND s.organization_id = ?4
           AND sm.role = ?5 AND sm.state = ?6 AND sm.revision = ?7
           AND sm.last_operation_id = ?8 AND m.state = 'active'
       ) THEN 1 ELSE 0 END
