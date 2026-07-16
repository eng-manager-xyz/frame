INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM organizations o
         JOIN organization_members m
           ON m.organization_id = o.id AND m.user_id = ?3 AND m.state = 'active'
         JOIN users u ON u.id = m.user_id AND u.status = 'active'
         JOIN auth_identities_v2 i ON i.user_id = u.id
         WHERE o.id = ?2
           AND o.status = 'active'
           AND i.identity_revision = ?4
           AND i.session_version = ?5
           AND (
             ?6 = 'any'
             OR (?6 = 'admin' AND m.role IN ('owner', 'admin'))
           )
       ) THEN 1 ELSE 0 END
