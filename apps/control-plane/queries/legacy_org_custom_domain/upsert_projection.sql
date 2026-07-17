INSERT INTO legacy_org_custom_domain_projection_v1(
  organization_id,
  custom_domain,
  domain_verified_iso,
  source_row_digest,
  imported_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(organization_id) DO UPDATE SET
  custom_domain = excluded.custom_domain,
  domain_verified_iso = excluded.domain_verified_iso,
  source_row_digest = excluded.source_row_digest,
  imported_at_ms = excluded.imported_at_ms
