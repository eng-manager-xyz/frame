UPDATE users
SET legacy_onboarding_steps_json = json_set(
      COALESCE(legacy_onboarding_steps_json, '{}'),
      '$.customDomain', json('true')
    ),
    updated_at_ms = ?2,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?3
WHERE id = ?1;
