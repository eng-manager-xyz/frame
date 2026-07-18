UPDATE auth_verification_challenges_v2
SET identifier_key_version = ?4,
    identifier_digest = ?5,
    secret_key_version = ?6,
    secret_digest = ?7,
    attempt_count = ?8,
    consumed_at_ms = ?9,
    state = ?10,
    revision = revision + 1,
    last_operation_id = ?11
WHERE id = ?1
  AND revision = ?2
  AND state = ?3
