UPDATE legacy_organization_library_r2_effects_v1
SET effect_state = 'applied', applied_at_ms = ?3
WHERE operation_id = ?1 AND effect_order = ?2 AND effect_state = 'pending'
