INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1 FROM auth_identities_v2 WHERE user_id = ?1
       ) THEN 1 ELSE 0 END
