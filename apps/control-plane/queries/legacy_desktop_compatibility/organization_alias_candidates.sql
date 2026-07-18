SELECT DISTINCT organization_id, owner_user_id FROM (
  SELECT organization.id AS organization_id, organization.owner_id AS owner_user_id
  FROM organizations organization
  WHERE organization.status = 'active'
    AND organization.tombstoned_at_ms IS NULL
    AND organization.owner_id = ?1
  UNION ALL
  SELECT organization.id AS organization_id, organization.owner_id AS owner_user_id
  FROM organization_members membership
  JOIN organizations organization ON organization.id = membership.organization_id
  WHERE membership.user_id = ?1
    AND membership.state = 'active'
    AND organization.status = 'active'
    AND organization.tombstoned_at_ms IS NULL
)
ORDER BY organization_id
LIMIT 512;
