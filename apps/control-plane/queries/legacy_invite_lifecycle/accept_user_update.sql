UPDATE users
SET legacy_onboarding_steps_json = json_set(
      COALESCE(legacy_onboarding_steps_json, '{}'),
      '$.organizationSetup', json('true'),
      '$.customDomain', json('true'),
      '$.inviteTeam', json('true')
    ),
    active_organization_id = ?2,
    default_organization_id = CASE
      WHEN default_organization_id IS NULL OR default_organization_id = '' THEN ?2
      ELSE default_organization_id
    END,
    legacy_third_party_stripe_subscription_id = CASE
      WHEN ?3 = 1 THEN ?4
      ELSE legacy_third_party_stripe_subscription_id
    END,
    organization_preference_revision = organization_preference_revision + 1,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    updated_at_ms = ?5
WHERE id = ?1
  AND status = 'active'
  AND deleted_at_ms IS NULL;
