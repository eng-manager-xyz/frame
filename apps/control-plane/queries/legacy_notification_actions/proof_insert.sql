INSERT INTO legacy_notification_action_proof_consumptions_v1(
  mutation_grant_id, session_id, actor_id, related_operation_id,
  tenant_kind, tenant_id, organization_id, action, request_digest,
  outcome, consumed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
