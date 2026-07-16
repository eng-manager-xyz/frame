INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM users u
         JOIN auth_identities_v2 i ON i.user_id = u.id
         JOIN organizations o ON o.id = ?2
         JOIN organization_members m
           ON m.organization_id = o.id AND m.user_id = u.id
         WHERE u.id = ?3
           AND u.status = 'active'
           AND i.identity_revision = ?4
           AND i.session_version = ?5
           AND o.status = ?6
           AND o.revision = ?7
           AND o.authority_version = ?8
           AND m.state = 'active'
           AND m.revision = ?9
           AND m.authority_version = ?10
           AND (
             (?11 = 'any' AND m.role IN ('owner', 'admin', 'member', 'viewer'))
             OR (?11 = 'write' AND m.role IN ('owner', 'admin', 'member'))
             OR (?11 = 'admin' AND m.role IN ('owner', 'admin'))
             OR (?11 = 'owner' AND m.role = 'owner')
           )
       ) THEN 1 ELSE 0 END
