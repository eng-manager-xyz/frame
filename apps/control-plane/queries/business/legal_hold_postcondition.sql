INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM business_legal_holds_v1
  WHERE id=?2 AND organization_id=?3 AND data_class=?4 AND subject_id=?5
    AND reason_code=?6 AND placed_by_user_id=?7 AND placed_at_ms=?8
    AND released_at_ms IS ?9
) THEN 1 ELSE 0 END
