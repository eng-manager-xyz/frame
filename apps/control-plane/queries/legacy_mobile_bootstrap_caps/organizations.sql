SELECT
  organization.id AS mapped_organization_id,
  organization_alias.legacy_organization_id AS legacy_organization_id,
  COALESCE(organization.legacy_user_account_name, organization.name) AS name,
  COALESCE(organization.legacy_icon_key, organization.legacy_desktop_icon_url) AS icon_key,
  CASE
    WHEN organization.owner_id = ?1 THEN 'owner'
    WHEN membership.role = 'admin' THEN 'admin'
    ELSE 'member'
  END AS effective_role
FROM organizations organization
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = ?1
 AND membership.state = 'active'
WHERE organization.status = 'active'
  AND organization.tombstoned_at_ms IS NULL
  AND (organization.owner_id = ?1 OR membership.user_id IS NOT NULL);
