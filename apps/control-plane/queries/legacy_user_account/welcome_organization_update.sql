UPDATE organizations
SET name = ?2,
    legacy_user_account_name = ?3,
    updated_at_ms = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE id = ?1
  AND COALESCE(legacy_user_account_name, name) = 'My Organization';
