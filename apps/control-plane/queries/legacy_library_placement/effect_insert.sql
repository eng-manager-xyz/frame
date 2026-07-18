INSERT INTO authenticated_web_action_effects_v1(
  operation_id, organization_id, user_id, action,
  effect_state, value_json, created_at_ms
)
VALUES (?1, ?2, ?3, ?4, 'applied', ?5, ?6)
