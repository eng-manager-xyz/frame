SELECT organization.id AS organization_id
FROM legacy_user_account_organization_ids_v1 organization_alias
JOIN organizations organization
  ON organization.id = organization_alias.organization_id
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = ?1
 AND membership.state = 'active'
WHERE organization_alias.legacy_organization_id = ?2
  AND organization.id = ?3
  AND (organization.owner_id = ?1 OR membership.user_id IS NOT NULL)
LIMIT 2;
