INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM auth_identities_v2
         WHERE user_id = ?2 AND last_operation_id = ?3
       ) AND NOT EXISTS (
         SELECT 1 FROM auth_session_mutation_grants_v2 WHERE user_id = ?2
       ) THEN 1 ELSE 0 END
