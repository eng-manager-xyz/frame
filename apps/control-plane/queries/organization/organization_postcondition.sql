INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organizations o
         JOIN organization_members owner
           ON owner.organization_id = o.id AND owner.user_id = o.owner_id
         WHERE o.id = ?2
           AND o.status = ?3
           AND o.revision = ?4
           AND o.authority_version = ?5
           AND o.last_operation_id = ?6
           AND owner.role = 'owner' AND owner.state = 'active'
       ) AND (
         SELECT COUNT(*) FROM organization_members
         WHERE organization_id = ?2 AND role = 'owner' AND state = 'active'
       ) = 1 THEN 1 ELSE 0 END
