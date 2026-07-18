UPDATE users
SET active_organization_id = CASE
      WHEN active_organization_id = ?2 THEN COALESCE(?3, '')
      ELSE active_organization_id
    END,
    default_organization_id = CASE
      WHEN default_organization_id = ?2 THEN ?3
      ELSE default_organization_id
    END,
    legacy_third_party_stripe_subscription_id = CASE
      WHEN ?4 = 1 THEN NULL
      ELSE legacy_third_party_stripe_subscription_id
    END,
    organization_preference_revision = organization_preference_revision + CASE
      WHEN active_organization_id = ?2 OR default_organization_id = ?2 THEN 1
      ELSE 0
    END,
    legacy_user_account_revision = legacy_user_account_revision + CASE
      WHEN active_organization_id = ?2 OR default_organization_id = ?2 OR ?4 = 1 THEN 1
      ELSE 0
    END,
    updated_at_ms = CASE
      WHEN active_organization_id = ?2 OR default_organization_id = ?2 OR ?4 = 1 THEN ?5
      ELSE updated_at_ms
    END
WHERE id = ?1
  AND status = 'active'
  AND deleted_at_ms IS NULL
  AND ?6 = 1;
