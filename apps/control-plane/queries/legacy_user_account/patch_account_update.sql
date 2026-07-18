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
    default_organization_id = CASE WHEN ?6 = 1 THEN ?7 ELSE default_organization_id END,
    organization_preference_revision = organization_preference_revision + ?6,
    organization_last_operation_id = CASE WHEN ?6 = 1 THEN ?8 ELSE organization_last_operation_id END,
    updated_at_ms = ?9,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?8
WHERE id = ?1;
