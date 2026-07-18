UPDATE auth_sessions_v2
SET csrf_key_version = ?4,
    csrf_digest = ?5,
    revision = revision + 1,
    last_operation_id = ?6
WHERE id = ?1
  AND revision = ?2
  AND generation = ?3
  AND state = 'active'
