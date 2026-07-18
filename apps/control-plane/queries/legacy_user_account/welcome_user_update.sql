UPDATE users
SET display_name = ?2,
    legacy_last_name = ?3,
    legacy_onboarding_steps_json = json_set(
      COALESCE(legacy_onboarding_steps_json, '{}'), '$.welcome', json('true')
    ),
    updated_at_ms = ?4,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?5
WHERE id = ?1;
