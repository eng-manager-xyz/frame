INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?4,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_identities_v2 i
         JOIN users u ON u.id = i.user_id AND u.status = 'active'
         WHERE i.user_id = ?1
           AND i.identity_revision = ?2
           AND i.revision = ?3
       ) THEN 1 ELSE 0 END
