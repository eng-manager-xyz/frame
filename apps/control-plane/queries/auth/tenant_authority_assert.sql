INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?8,
       CASE WHEN EXISTS (
         SELECT 1
         FROM users u
         JOIN auth_identities_v2 i ON i.user_id = u.id
         JOIN organization_members m ON m.user_id = u.id
         JOIN organizations o ON o.id = m.organization_id
         WHERE u.id = ?1
           AND u.status = 'active'
           AND i.identity_revision = ?2
           AND i.revision = ?3
           AND m.organization_id = ?4
           AND m.state = 'active'
           AND m.role = ?5
           AND m.revision = ?6
           AND o.status = 'active'
           AND o.revision = ?7
       ) THEN 1 ELSE 0 END
