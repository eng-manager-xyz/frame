INSERT INTO organization_allowed_domains(
  organization_id, domain_ascii, verified_at_ms, created_at_ms, revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, 0, ?5)
ON CONFLICT(organization_id, domain_ascii) DO UPDATE SET
  verified_at_ms = excluded.verified_at_ms,
  revision = organization_allowed_domains.revision + 1,
  last_operation_id = excluded.last_operation_id
WHERE organization_allowed_domains.revision = ?6
