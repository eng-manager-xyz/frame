INSERT INTO legacy_mobile_session_challenges_v1(
  identifier_digest, token_digest, delivery_id, created_at_ms,
  expires_at_ms, request_operation_id
) VALUES(?1, ?2, ?3, ?4, ?4 + 600000, ?5)
ON CONFLICT(identifier_digest) DO UPDATE SET
  token_digest = excluded.token_digest,
  delivery_id = excluded.delivery_id,
  created_at_ms = excluded.created_at_ms,
  expires_at_ms = excluded.expires_at_ms,
  request_operation_id = excluded.request_operation_id
