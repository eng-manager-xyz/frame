UPDATE auth_identities_v2
SET session_version = session_version + 1,
    updated_at_ms = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE user_id = ?1
  AND revision = ?2
  AND session_version < 9007199254740991
  AND EXISTS (SELECT 1 FROM users u WHERE u.id = ?1 AND u.status = 'active')
