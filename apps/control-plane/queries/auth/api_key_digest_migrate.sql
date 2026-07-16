UPDATE auth_api_keys_v2
SET key_version = ?4,
    key_digest = ?5,
    revision = revision + 1,
    last_operation_id = ?6
WHERE id = ?1
  AND revision = ?2
  AND key_version = ?3
  AND revoked_at_ms IS NULL
