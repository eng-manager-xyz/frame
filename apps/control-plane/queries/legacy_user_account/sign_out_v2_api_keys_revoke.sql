UPDATE auth_api_keys_v2
SET revoked_at_ms = COALESCE(revoked_at_ms, ?2),
    revision = revision + CASE WHEN revoked_at_ms IS NULL THEN 1 ELSE 0 END,
    last_operation_id = CASE WHEN revoked_at_ms IS NULL THEN ?3 ELSE last_operation_id END
WHERE owner_id = ?1;
