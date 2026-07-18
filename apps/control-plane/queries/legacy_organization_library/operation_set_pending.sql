UPDATE legacy_organization_library_operations_v1
SET organization_id = ?2,
    state = 'storage_pending',
    result_json = ?3,
    effects_json = ?4,
    updated_at_ms = ?5
WHERE operation_id = ?1 AND state = 'claimed'
