INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organization_allowed_domains
         WHERE organization_id = ?2 AND domain_ascii = ?3
       ) OR (
         SELECT COUNT(*) FROM organization_allowed_domains WHERE organization_id = ?2
       ) < 256 THEN 1 ELSE 0 END
