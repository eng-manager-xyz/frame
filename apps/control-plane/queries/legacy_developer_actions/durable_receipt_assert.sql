INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'durable_receipt', 1,
  (SELECT COUNT(*)
   FROM legacy_developer_action_operations_v1 operation
   JOIN legacy_developer_action_receipts_v1 receipt
     ON receipt.operation_id = operation.operation_id
   JOIN legacy_developer_action_effects_v1 effect
     ON effect.operation_id = operation.operation_id
   WHERE operation.operation_id = ?1 AND operation.actor_id = ?2
     AND operation.action = ?3 AND operation.request_digest = ?4
     AND operation.state = 'complete' AND receipt.result_kind = ?5
     AND EXISTS (SELECT 1 FROM legacy_developer_action_audit_events_v1 audit
       WHERE audit.operation_id = operation.operation_id
         AND audit.actor_id = operation.actor_id AND audit.action = operation.action
         AND audit.outcome = 'allow')
     AND EXISTS (SELECT 1 FROM legacy_developer_action_proof_consumptions_v1 proof
       WHERE proof.mutation_grant_id = ?6 AND proof.session_id = ?7
         AND proof.actor_id = operation.actor_id
         AND proof.related_operation_id = operation.operation_id
         AND proof.action = operation.action
         AND proof.request_digest = operation.request_digest
         AND proof.outcome = ?8))
)
