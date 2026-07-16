INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM organization_members m
         JOIN users u ON u.id = m.user_id AND u.status = 'active'
         JOIN auth_identities_v2 i ON i.user_id = u.id
         WHERE m.organization_id = ?2
           AND m.user_id = ?3
           AND m.state = 'active'
           AND m.role <> 'owner'
           AND m.revision = ?4
       ) THEN 1 ELSE 0 END
