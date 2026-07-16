INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM folders f
         WHERE f.id = ?2 AND f.organization_id = ?3 AND f.space_id = ?4
           AND f.parent_id IS ?5 AND f.revision = ?6 AND f.last_operation_id = ?7
           AND f.depth <= 32
           AND EXISTS (
             SELECT 1 FROM organization_folder_closure_v1 self
             WHERE self.organization_id = ?3 AND self.space_id = ?4
               AND self.ancestor_id = f.id AND self.descendant_id = f.id AND self.distance = 0
           )
       ) AND NOT EXISTS (
         SELECT 1 FROM organization_folder_closure_v1
         WHERE organization_id = ?3 AND space_id = ?4
           AND ancestor_id = descendant_id AND distance <> 0
       ) THEN 1 ELSE 0 END
