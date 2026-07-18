SELECT organization_id, domain_ascii, verified_at_ms, created_at_ms, revision
FROM organization_allowed_domains
WHERE organization_id = ?1
  AND (?2 IS NULL OR domain_ascii > ?2 COLLATE NOCASE)
ORDER BY domain_ascii COLLATE NOCASE
LIMIT ?3
