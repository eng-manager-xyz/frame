SELECT
  organization.id AS organization_id,
  organization_alias.legacy_organization_id,
  COALESCE(organization.legacy_user_account_name, organization.name) AS name,
  organization.owner_id,
  owner_alias.legacy_user_id AS legacy_owner_id,
  organization.status,
  organization.tombstoned_at_ms,
  organization.legacy_desktop_metadata_json AS metadata_json,
  COALESCE(organization.legacy_desktop_icon_url, organization.legacy_icon_key) AS icon_url,
  organization.revision,
  organization.legacy_desktop_branding_revision AS branding_revision,
  CASE
    WHEN organization.owner_id = ?1 THEN 'owner'
    WHEN membership.state = 'active' AND membership.role = 'admin' THEN 'admin'
    WHEN membership.state = 'active' AND membership.role IN ('owner','member','viewer') THEN 'member'
    ELSE NULL
  END AS effective_role
FROM organizations organization
LEFT JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
LEFT JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = organization.owner_id
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = ?1
WHERE organization.id = ?2 OR organization_alias.legacy_organization_id = ?2
ORDER BY CASE WHEN organization_alias.legacy_organization_id = ?2 THEN 0 ELSE 1 END
LIMIT 2;
