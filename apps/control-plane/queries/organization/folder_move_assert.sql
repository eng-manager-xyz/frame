INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM folders moved
         WHERE moved.id = ?2
           AND moved.organization_id = ?3
           AND moved.space_id = ?4
           AND moved.deleted_at_ms IS NULL
           AND moved.revision = ?5
           AND moved.tree_revision = ?6
           AND EXISTS (
             SELECT 1
             FROM organization_members om
             LEFT JOIN space_members sm
               ON sm.space_id = moved.space_id
              AND sm.user_id = ?9
              AND sm.state = 'active'
             WHERE om.organization_id = moved.organization_id
               AND om.user_id = ?9
               AND om.state = 'active'
               AND (
                 om.role IN ('owner', 'admin')
                 OR (
                   om.role = 'member'
                   AND (
                     sm.role = 'manager'
                     OR (sm.role = 'contributor' AND moved.created_by_user_id = ?9)
                   )
                 )
               )
           )
           AND (?7 IS NULL OR EXISTS (
             SELECT 1 FROM folders parent
             WHERE parent.id = ?7
               AND parent.organization_id = ?3
               AND parent.space_id = ?4
               AND parent.deleted_at_ms IS NULL
               AND parent.revision = ?8
               AND parent.depth + 1 <= 32
           ))
           AND (?7 IS NULL OR NOT EXISTS (
             SELECT 1 FROM organization_folder_closure_v1 cycle
             WHERE cycle.organization_id = ?3 AND cycle.space_id = ?4
               AND cycle.ancestor_id = moved.id AND cycle.descendant_id = ?7
           ))
           AND NOT EXISTS (
             SELECT 1
             FROM organization_folder_closure_v1 subtree
             JOIN folders descendant ON descendant.id = subtree.descendant_id
             WHERE subtree.organization_id = ?3 AND subtree.space_id = ?4
               AND subtree.ancestor_id = moved.id
               AND descendant.depth
                 + COALESCE((SELECT depth + 1 FROM folders WHERE id = ?7), 0)
                 - moved.depth > 32
           )
       ) THEN 1 ELSE 0 END
