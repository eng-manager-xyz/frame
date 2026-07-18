INSERT INTO legacy_mobile_session_operations_v1(
  operation_id, action, actor_id, subject_digest, provider_effect,
  state, created_at_ms, completed_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, 'complete', ?6, ?6)
