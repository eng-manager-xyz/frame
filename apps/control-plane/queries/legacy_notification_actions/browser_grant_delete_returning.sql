DELETE FROM auth_session_mutation_grants_v2
WHERE id = ?1
  AND session_id = ?2
  AND user_id = ?3
RETURNING id AS mutation_grant_id, session_id, user_id AS actor_id
