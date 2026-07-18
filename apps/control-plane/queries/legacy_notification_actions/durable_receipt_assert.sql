INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'durable_receipt',
  1,
  (
    SELECT COUNT(*)
    FROM legacy_notification_action_operations_v1 operation
    JOIN legacy_notification_action_receipts_v1 receipt
      ON receipt.operation_id = operation.operation_id
    JOIN legacy_notification_action_effects_v1 effect
      ON effect.operation_id = operation.operation_id
     AND effect.actor_id = operation.actor_id
     AND effect.organization_id IS operation.organization_id
     AND effect.action = operation.action
    WHERE operation.operation_id = ?1
      AND operation.tenant_kind = ?2
      AND operation.tenant_id = ?3
      AND operation.organization_id IS ?4
      AND operation.actor_id = ?5
      AND operation.action = ?6
      AND operation.request_digest = ?7
      AND operation.state = 'complete'
      AND receipt.result_kind = ?8
      AND receipt.selected_notification_id IS ?9
      AND receipt.matched_count IS ?10
      AND receipt.read_at_ms IS ?11
      AND receipt.notifications_json IS ?12
      AND receipt.preserved_before_sha256 IS ?13
      AND receipt.preserved_after_sha256 IS ?14
      AND receipt.matching_before = ?15
      AND receipt.updated_rows = ?16
      AND receipt.matching_after = ?17
      AND receipt.out_of_scope_updated_rows = ?18
      AND receipt.other_actor_rows_updated = ?19
      AND effect.value_json = ?20
      AND EXISTS (
        SELECT 1
        FROM legacy_notification_action_audit_events_v1 audit
        WHERE audit.operation_id = operation.operation_id
          AND audit.actor_id = operation.actor_id
          AND audit.organization_id IS operation.organization_id
          AND audit.action = operation.action
          AND audit.outcome = 'allow'
      )
      AND EXISTS (
        SELECT 1
        FROM legacy_notification_action_proof_consumptions_v1 proof
        WHERE proof.mutation_grant_id = ?21
          AND proof.session_id = ?22
          AND proof.actor_id = operation.actor_id
          AND proof.related_operation_id = operation.operation_id
          AND proof.tenant_kind = operation.tenant_kind
          AND proof.tenant_id = operation.tenant_id
          AND proof.organization_id IS operation.organization_id
          AND proof.action = operation.action
          AND proof.request_digest = operation.request_digest
          AND proof.outcome = ?23
      )
  )
)
