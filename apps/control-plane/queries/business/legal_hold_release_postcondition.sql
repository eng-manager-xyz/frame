INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM business_legal_holds_v1
  WHERE id=?2 AND organization_id=?3 AND released_at_ms=?4
) THEN 1 ELSE 0 END
