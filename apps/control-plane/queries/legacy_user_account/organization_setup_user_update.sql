UPDATE users
SET active_organization_id = ?2,
    default_organization_id = ?2,
    organization_preference_revision = organization_preference_revision + 1,
    organization_last_operation_id = ?3,
    legacy_onboarding_steps_json = json_set(
      COALESCE(legacy_onboarding_steps_json, '{}'),
      '$.organizationSetup', json('true')
    ),
    updated_at_ms = ?4,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?3
WHERE id = ?1;
