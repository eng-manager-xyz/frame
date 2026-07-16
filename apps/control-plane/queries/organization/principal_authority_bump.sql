UPDATE auth_identities_v2
SET session_version = session_version + 1,
    revision = revision + 1,
    updated_at_ms = ?2,
    last_operation_id = ?3
WHERE user_id = ?1
