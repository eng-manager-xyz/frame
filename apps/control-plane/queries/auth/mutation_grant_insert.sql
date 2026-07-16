INSERT INTO auth_session_mutation_grants_v2(
  id, session_id, user_id, generation,
  token_key_version, token_digest, created_at_ms, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
