SELECT
  operation.operation_id,
  operation.request_digest,
  operation.state,
  receipt.result_kind,
  receipt.selected_notification_id,
  receipt.matched_count,
  receipt.read_at_ms,
  receipt.notifications_json,
  receipt.preserved_before_sha256,
  receipt.preserved_after_sha256,
  receipt.matching_before,
  receipt.updated_rows,
  receipt.matching_after,
  receipt.out_of_scope_updated_rows,
  receipt.other_actor_rows_updated,
  effect.value_json AS effect_json,
  (
    SELECT COUNT(*)
    FROM legacy_notification_action_audit_events_v1 audit
    WHERE audit.operation_id = operation.operation_id
      AND audit.actor_id = operation.actor_id
      AND audit.organization_id IS operation.organization_id
      AND audit.action = operation.action
      AND audit.outcome = 'allow'
  ) AS audit_count,
  (
    SELECT COUNT(*)
    FROM legacy_notification_action_proof_consumptions_v1 proof
    WHERE proof.related_operation_id = operation.operation_id
      AND proof.actor_id = operation.actor_id
      AND proof.action = operation.action
      AND proof.outcome IN ('applied', 'replay')
  ) AS proof_count
FROM legacy_notification_action_operations_v1 operation
LEFT JOIN legacy_notification_action_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
LEFT JOIN legacy_notification_action_effects_v1 effect
  ON effect.operation_id = operation.operation_id
 AND effect.actor_id = operation.actor_id
 AND effect.organization_id IS operation.organization_id
 AND effect.action = operation.action
WHERE operation.tenant_kind = ?1
  AND operation.tenant_id = ?2
  AND operation.actor_id = ?3
  AND operation.action = ?4
  AND operation.idempotency_key_digest = ?5
LIMIT 2
