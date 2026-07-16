INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM spaces s
         JOIN organization_members om
           ON om.organization_id = s.organization_id AND om.user_id = ?3 AND om.state = 'active'
         LEFT JOIN space_members sm
           ON sm.space_id = s.id AND sm.user_id = ?3 AND sm.state = 'active'
         WHERE s.id = ?2
           AND s.organization_id = ?4
           AND s.deleted_at_ms IS NULL
           AND s.revision = ?5
           AND (
             om.role IN ('owner', 'admin')
             OR (
               om.role = 'member' AND ?6 IS NOT NULL AND sm.revision = ?6
               AND (
                 (?7 = 'manager' AND sm.role = 'manager')
                 OR (?7 = 'write' AND sm.role IN ('manager', 'contributor'))
               )
             )
           )
       ) THEN 1 ELSE 0 END
