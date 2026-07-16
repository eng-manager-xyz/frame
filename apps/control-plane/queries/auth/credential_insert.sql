INSERT INTO auth_session_credentials_v2(
  key_version, digest, session_id, family_id, state, revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, 'current', 0, ?5)
