INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM folders managed
         JOIN organization_members om
           ON om.organization_id = managed.organization_id
          AND om.user_id = ?7
          AND om.state = 'active'
         LEFT JOIN space_members sm
           ON sm.space_id = managed.space_id
          AND sm.user_id = ?7
          AND sm.state = 'active'
         WHERE managed.id = ?2
           AND managed.organization_id = ?3
           AND managed.space_id = ?4
           AND managed.revision = ?5
           AND managed.tree_revision = ?6
           AND managed.deleted_at_ms IS NULL
           AND (
             om.role IN ('owner', 'admin')
             OR (
               om.role = 'member'
               AND (
                 sm.role = 'manager'
                 OR (sm.role = 'contributor' AND managed.created_by_user_id = ?7)
               )
             )
           )
       ) THEN 1 ELSE 0 END
