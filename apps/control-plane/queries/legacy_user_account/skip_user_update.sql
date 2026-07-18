UPDATE users
SET display_name = ?2,
    legacy_onboarding_steps_json = json_object(
      'welcome', json('true'),
      'organizationSetup', json('true'),
      'customDomain', json('true'),
      'inviteTeam', json('true'),
      'download', json('true')
    ),
    active_organization_id = CASE WHEN ?3 = 1 THEN ?4 ELSE active_organization_id END,
    default_organization_id = CASE WHEN ?3 = 1 THEN ?4 ELSE default_organization_id END,
    organization_preference_revision = organization_preference_revision + ?3,
    organization_last_operation_id = CASE WHEN ?3 = 1 THEN ?5 ELSE organization_last_operation_id END,
    updated_at_ms = ?6,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?5
WHERE id = ?1;
