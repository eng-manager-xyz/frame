SELECT CASE WHEN o.id IS NULL THEN 0 ELSE 1 END AS organization_present,
       CASE WHEN p.organization_id IS NULL THEN 0 ELSE 1 END AS projection_present,
       p.custom_domain,
       p.domain_verified_iso AS domain_verified
FROM users u
LEFT JOIN organizations o
  ON o.id = u.active_organization_id
LEFT JOIN legacy_org_custom_domain_projection_v1 p
  ON p.organization_id = o.id
WHERE u.id = ?1
LIMIT 1
