UPDATE legacy_organization_library_operations_v1
SET organization_id = ?2,
    state = 'complete',
    result_json = COALESCE(result_json, ?3),
    effects_json = COALESCE(effects_json, ?4),
    updated_at_ms = ?5,
    completed_at_ms = ?5
WHERE operation_id = ?1 AND state IN ('claimed', 'storage_pending')
  AND NOT EXISTS (
    SELECT 1 FROM legacy_organization_library_r2_effects_v1 effect
    WHERE effect.operation_id = ?1 AND effect.effect_state <> 'applied'
  )
