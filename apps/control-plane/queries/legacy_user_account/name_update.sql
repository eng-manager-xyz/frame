UPDATE users
SET display_name = CASE ?2
      WHEN 0 THEN display_name
      WHEN 1 THEN NULL
      ELSE ?3
    END,
    legacy_last_name = CASE ?4
      WHEN 0 THEN legacy_last_name
      WHEN 1 THEN NULL
      ELSE ?5
    END,
    updated_at_ms = ?6,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?7
WHERE id = ?1;
