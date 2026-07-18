SELECT effect_order, effect_kind, object_key, checksum_sha256, content_type
FROM legacy_organization_library_r2_effects_v1
WHERE operation_id = ?1 AND effect_state = 'pending'
ORDER BY effect_order
LIMIT 10001
