SELECT COUNT(*) AS assigned_count
FROM organization_members membership
JOIN organizations organization ON organization.id = membership.organization_id
JOIN users owner ON owner.id = organization.owner_id
WHERE membership.organization_id = ?1
  AND membership.state = 'active'
  AND (
    membership.has_pro_seat = 1
    OR (
      membership.user_id = organization.owner_id
      AND owner.legacy_stripe_subscription_id IS NOT NULL
      AND owner.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')
    )
  )
