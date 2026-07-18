SELECT projection.custom_domain,
       projection.domain_verified_iso
FROM organizations organization
LEFT JOIN legacy_org_custom_domain_projection_v1 projection
  ON projection.organization_id = organization.id
WHERE organization.owner_id = ?1
LIMIT 1;
