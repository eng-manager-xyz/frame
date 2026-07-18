UPDATE auth_api_keys_v2
SET revoked_at_ms = COALESCE(revoked_at_ms, ?3),
    revision = revision + 1,
    last_operation_id = ?4
WHERE id = ?1
  AND revision = ?2
  AND revoked_at_ms IS NULL
