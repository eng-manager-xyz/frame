SELECT
  organization_alias.legacy_organization_id,
  COALESCE(organization.legacy_user_account_name, organization.name) AS name,
  owner_alias.legacy_user_id AS legacy_owner_id,
  CASE
    WHEN organization.owner_id = ?1 THEN 'owner'
    WHEN membership.role = 'admin' THEN 'admin'
    WHEN membership.role IN ('owner', 'member', 'viewer') THEN 'member'
    ELSE NULL
  END AS effective_role,
  COALESCE(organization.legacy_desktop_icon_url, organization.legacy_icon_key) AS icon_url,
  organization.legacy_desktop_metadata_json AS metadata_json,
  organization.created_at_ms
FROM organizations organization
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = organization.owner_id
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = ?1
 AND membership.state = 'active'
WHERE organization.status = 'active'
  AND organization.tombstoned_at_ms IS NULL
  AND (organization.owner_id = ?1 OR membership.user_id IS NOT NULL)
ORDER BY
  CASE WHEN organization.owner_id = ?1 THEN 0 ELSE 1 END,
  organization.created_at_ms,
  organization.id
LIMIT 512;
