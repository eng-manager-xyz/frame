UPDATE auth_session_credentials_v2
SET key_version = ?5,
    digest = ?6,
    revision = revision + 1,
    last_operation_id = ?7
WHERE key_version = ?1
  AND digest = ?2
  AND session_id = ?3
  AND revision = ?4
  AND state = 'current'
