INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN NOT EXISTS (
  SELECT 1 FROM organizations WHERE id = ?2
) THEN 1 ELSE 0 END
