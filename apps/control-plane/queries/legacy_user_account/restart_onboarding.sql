UPDATE users
SET legacy_onboarding_steps_json = NULL,
    display_name = NULL,
    legacy_last_name = NULL,
    legacy_onboarding_completed_at_ms = NULL,
    updated_at_ms = ?2,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?3
WHERE id = ?1;
