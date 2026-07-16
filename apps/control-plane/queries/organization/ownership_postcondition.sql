INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN (
         SELECT COUNT(*) FROM organization_members
         WHERE organization_id = ?2 AND role = 'owner' AND state = 'active'
       ) = 1 AND EXISTS (
         SELECT 1 FROM organizations o
         JOIN organization_members m
           ON m.organization_id = o.id AND m.user_id = o.owner_id
         WHERE o.id = ?2 AND o.owner_id = ?3 AND m.role = 'owner' AND m.state = 'active'
           AND o.last_operation_id = ?4
       ) THEN 1 ELSE 0 END
