SELECT owner.legacy_stripe_subscription_id AS subscription_id
FROM organization_members membership
JOIN organizations organization
  ON organization.id = membership.organization_id
 AND organization.id <> ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN users owner ON owner.id = organization.owner_id
WHERE membership.user_id = ?1
  AND membership.state = 'active'
  AND membership.has_pro_seat = 1
ORDER BY organization.id
LIMIT 1
