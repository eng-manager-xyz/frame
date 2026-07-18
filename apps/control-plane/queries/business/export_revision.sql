SELECT COUNT(*) AS source_revision
FROM business_repository_operations_v1
WHERE organization_id = ?1
