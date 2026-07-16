INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM users u
         JOIN auth_identities_v2 i ON i.user_id = u.id
         WHERE u.id = ?2
           AND u.status = 'active'
           AND i.identity_revision = ?3
           AND i.session_version = ?4
       ) THEN 1 ELSE 0 END
