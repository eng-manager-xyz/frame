INSERT INTO authenticated_web_action_operations_v1(
  operation_id, organization_id, user_id, action, idempotency_key,
  request_digest, state, response_json, created_at_ms, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'claimed', NULL, ?7, NULL)
