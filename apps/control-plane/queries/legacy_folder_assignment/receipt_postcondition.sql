UPDATE authenticated_web_action_assertions_v1
SET actual_count = (
  SELECT COUNT(*)
  FROM authenticated_web_action_operations_v1 operation
  JOIN authenticated_web_action_effects_v1 effect
    ON effect.operation_id = operation.operation_id
   AND effect.organization_id = operation.organization_id
   AND effect.user_id = operation.user_id
   AND effect.action = operation.action
   AND effect.effect_state = 'applied'
  WHERE operation.operation_id = ?1
    AND operation.organization_id = ?2
    AND operation.user_id = ?3
    AND operation.action = ?4
    AND operation.state = 'complete'
    AND operation.response_json = ?5
    AND effect.value_json = ?6
    AND EXISTS (
      SELECT 1
      FROM business_audit_events_v1 audit
      WHERE audit.operation_id = operation.operation_id
        AND audit.organization_id = operation.organization_id
        AND audit.action = ?7
        AND audit.outcome = 'allow'
    )
)
WHERE operation_id = ?1
  AND assertion_kind = 'action_effect'
