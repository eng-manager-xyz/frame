UPDATE auth_sessions_v2
SET token_key_version = ?4,
    token_digest = ?5,
    csrf_key_version = COALESCE(?6, csrf_key_version),
    csrf_digest = COALESCE(?7, csrf_digest),
    revision = revision + 1,
    last_operation_id = ?8
WHERE id = ?1
  AND revision = ?2
  AND generation = ?3
  AND state = 'active'
