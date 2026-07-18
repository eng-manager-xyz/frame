INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organization_members
         WHERE organization_id = ?2 AND user_id = ?3 AND role = ?4 AND state = ?5
           AND revision = ?6 AND last_operation_id = ?7
       ) AND EXISTS (
         SELECT 1 FROM organizations o JOIN organization_members m
           ON m.organization_id = o.id AND m.user_id = o.owner_id
         WHERE o.id = ?2 AND m.role = 'owner' AND m.state = 'active'
       ) THEN 1 ELSE 0 END
