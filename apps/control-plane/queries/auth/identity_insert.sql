INSERT INTO auth_identities_v2(
  user_id, identity_revision, session_version,
  created_at_ms, updated_at_ms, revision, last_operation_id
) VALUES (?1, ?2, 0, ?3, ?3, 0, ?4)
