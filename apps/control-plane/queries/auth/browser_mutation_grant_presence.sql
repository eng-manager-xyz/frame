SELECT 1 AS present
FROM auth_session_mutation_grants_v2
WHERE id = ?1
  AND session_id = ?2
  AND user_id = ?3
LIMIT 1;
