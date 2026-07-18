SELECT
  operation.operation_id,
  operation.request_digest,
  operation.state,
  operation.response_json,
  effect.value_json AS effect_json,
  (
    SELECT COUNT(*)
    FROM business_audit_events_v1 audit
    WHERE audit.operation_id = operation.operation_id
      AND audit.organization_id = operation.organization_id
      AND audit.action = 'legacy.library_placement'
      AND audit.outcome = 'allow'
  ) AS audit_count
FROM authenticated_web_action_operations_v1 operation
LEFT JOIN authenticated_web_action_effects_v1 effect
  ON effect.operation_id = operation.operation_id
 AND effect.organization_id = operation.organization_id
 AND effect.user_id = operation.user_id
 AND effect.action = operation.action
 AND effect.effect_state = 'applied'
WHERE operation.organization_id = ?1
  AND operation.user_id = ?2
  AND operation.action = ?3
  AND operation.idempotency_key = ?4
LIMIT 2
