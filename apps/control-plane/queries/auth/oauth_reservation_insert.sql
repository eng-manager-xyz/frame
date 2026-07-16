INSERT INTO auth_oauth_reservations_v2(
  id, flow_id, provider,
  initiator_session_id, initiator_user_id, initiator_generation,
  expires_at_ms, created_at_ms, consumed_at_ms, revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0, ?9)
