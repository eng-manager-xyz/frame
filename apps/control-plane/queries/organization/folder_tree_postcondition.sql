INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM organization_folder_closure_v1 subtree
         JOIN folders descendant ON descendant.id = subtree.descendant_id
         WHERE subtree.organization_id = ?2 AND subtree.space_id = ?3
           AND subtree.ancestor_id = ?4
           AND (
             descendant.organization_id <> ?2
             OR descendant.space_id <> ?3
             OR descendant.deleted_at_ms IS NOT NULL
             OR descendant.depth <> (
               SELECT COUNT(*) FROM organization_folder_closure_v1 ancestors
               WHERE ancestors.organization_id = ?2 AND ancestors.space_id = ?3
                 AND ancestors.descendant_id = descendant.id
                 AND ancestors.ancestor_id <> descendant.id
             )
             OR descendant.depth > 32
           )
       ) THEN 1 ELSE 0 END
