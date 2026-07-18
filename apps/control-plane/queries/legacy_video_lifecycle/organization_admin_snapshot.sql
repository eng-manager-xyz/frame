SELECT
  organization.id AS organization_id,
  organization.legacy_icon_key AS existing_icon_key
FROM organizations organization
LEFT JOIN legacy_user_account_organization_ids_v1 legacy
  ON legacy.organization_id = organization.id
JOIN organization_members member
  ON member.organization_id = organization.id
 AND member.user_id = ?1
 AND member.state = 'active'
 AND member.role IN ('owner', 'admin')
WHERE (organization.id = ?2 OR legacy.legacy_organization_id = ?2)
  AND organization.status = 'active'
LIMIT 2;
