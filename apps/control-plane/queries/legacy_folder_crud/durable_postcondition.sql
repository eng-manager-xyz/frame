INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'durable_receipt', 1,
  (
    SELECT COUNT(*)
    FROM legacy_folder_crud_operations_v1 operation
    JOIN legacy_folder_crud_receipts_v1 receipt
      ON receipt.operation_id = operation.operation_id
    JOIN legacy_folder_crud_effects_v1 effect
      ON effect.operation_id = operation.operation_id
     AND effect.organization_id = operation.organization_id
     AND effect.actor_id = operation.actor_id
    JOIN legacy_folder_crud_audit_events_v1 audit
      ON audit.operation_id = operation.operation_id
     AND audit.organization_id = operation.organization_id
     AND audit.actor_id = operation.actor_id
     AND audit.source_operation_id = operation.source_operation_id
     AND audit.outcome = 'allow'
    WHERE operation.operation_id = ?1
      AND operation.organization_id = ?2
      AND operation.actor_id = ?3
      AND operation.source_operation_id = ?4
      AND operation.request_digest = ?5
      AND operation.state = 'complete'
      AND operation.completed_at_ms = ?6
      AND receipt.mutation_kind = ?7
      AND receipt.folder_id = ?8
      AND effect.mutation_kind = ?7
      AND effect.affected_folder_count = receipt.affected_folder_count
  )
)
