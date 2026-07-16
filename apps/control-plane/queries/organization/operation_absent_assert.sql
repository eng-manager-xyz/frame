INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN NOT EXISTS (
  SELECT 1 FROM organization_repository_operations_v1
  WHERE operation_id = ?2 OR (organization_id = ?3 AND idempotency_key = ?4)
) THEN 1 ELSE 0 END
