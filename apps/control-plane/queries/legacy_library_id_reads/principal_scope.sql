SELECT
  actor.id AS actor_id,
  organization.id AS active_organization_id,
  organization_alias.legacy_organization_id AS active_legacy_organization_id
FROM users actor
JOIN organizations organization
  ON organization.id = actor.active_organization_id
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = actor.id
 AND membership.state = 'active'
WHERE actor.id = ?1
  AND (organization.owner_id = actor.id OR membership.user_id IS NOT NULL)
LIMIT 2;
