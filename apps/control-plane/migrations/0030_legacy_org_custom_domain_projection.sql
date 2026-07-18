PRAGMA foreign_keys = ON;

-- Lossless compatibility projection for Cap organizations.customDomain and
-- organizations.domainVerified. The latter is a Drizzle Date at runtime and
-- JSON.stringify emits its ISO-8601 string; it is deliberately not collapsed
-- into the boolean type claimed by the stale client declarations.
CREATE TABLE legacy_org_custom_domain_projection_v1 (
  organization_id TEXT PRIMARY KEY NOT NULL
    REFERENCES organizations(id) ON DELETE CASCADE,
  custom_domain TEXT,
  domain_verified_iso TEXT,
  source_row_digest TEXT NOT NULL CHECK (
    length(source_row_digest) = 64
      AND lower(source_row_digest) = source_row_digest
      AND source_row_digest NOT GLOB '*[^0-9a-f]*'
  ),
  imported_at_ms INTEGER NOT NULL CHECK (
    imported_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (custom_domain IS NULL OR length(custom_domain) <= 255),
  CHECK (
    domain_verified_iso IS NULL OR (
      length(domain_verified_iso) = 24
        AND substr(domain_verified_iso, 5, 1) = '-'
        AND substr(domain_verified_iso, 8, 1) = '-'
        AND substr(domain_verified_iso, 11, 1) = 'T'
        AND substr(domain_verified_iso, 14, 1) = ':'
        AND substr(domain_verified_iso, 17, 1) = ':'
        AND substr(domain_verified_iso, 20, 1) = '.'
        AND substr(domain_verified_iso, 24, 1) = 'Z'
        AND substr(domain_verified_iso, 1, 4) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 6, 2) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 9, 2) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 12, 2) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 15, 2) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 18, 2) NOT GLOB '*[^0-9]*'
        AND substr(domain_verified_iso, 21, 3) NOT GLOB '*[^0-9]*'
        AND CAST(substr(domain_verified_iso, 6, 2) AS INTEGER) BETWEEN 1 AND 12
        AND CAST(substr(domain_verified_iso, 9, 2) AS INTEGER) BETWEEN 1 AND 31
        AND CAST(substr(domain_verified_iso, 12, 2) AS INTEGER) BETWEEN 0 AND 23
        AND CAST(substr(domain_verified_iso, 15, 2) AS INTEGER) BETWEEN 0 AND 59
        AND CAST(substr(domain_verified_iso, 18, 2) AS INTEGER) BETWEEN 0 AND 59
    )
  )
);

CREATE INDEX legacy_org_custom_domain_projection_import_idx
  ON legacy_org_custom_domain_projection_v1(imported_at_ms, organization_id);
