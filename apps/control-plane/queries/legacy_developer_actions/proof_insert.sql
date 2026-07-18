INSERT INTO legacy_developer_action_proof_consumptions_v1(
  mutation_grant_id, session_id, actor_id, related_operation_id,
  action, request_digest, outcome, consumed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
