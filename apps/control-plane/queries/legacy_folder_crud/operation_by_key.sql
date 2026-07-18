SELECT
  operation.operation_id,
  operation.request_digest,
  operation.state,
  receipt.result_kind,
  receipt.mutation_kind,
  receipt.folder_id,
  receipt.legacy_folder_id,
  receipt.name,
  receipt.color,
  receipt.affected_folder_count,
  (SELECT COUNT(*) FROM legacy_folder_crud_effects_v1 effect
   WHERE effect.operation_id = operation.operation_id) AS effect_count,
  (SELECT COUNT(*) FROM legacy_folder_crud_audit_events_v1 audit
   WHERE audit.operation_id = operation.operation_id
     AND audit.organization_id = operation.organization_id
     AND audit.actor_id = operation.actor_id
     AND audit.source_operation_id = operation.source_operation_id
     AND audit.outcome = 'allow') AS audit_count
FROM legacy_folder_crud_operations_v1 operation
LEFT JOIN legacy_folder_crud_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
WHERE operation.organization_id = ?1
  AND operation.actor_id = ?2
  AND operation.source_operation_id = ?3
  AND operation.idempotency_key_digest = ?4
LIMIT 2
