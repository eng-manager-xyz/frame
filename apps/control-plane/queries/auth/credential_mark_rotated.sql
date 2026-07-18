UPDATE auth_session_credentials_v2
SET state = 'rotated',
    revision = revision + 1,
    last_operation_id = ?5
WHERE key_version = ?1
  AND digest = ?2
  AND session_id = ?3
  AND revision = ?4
  AND state = 'current'
