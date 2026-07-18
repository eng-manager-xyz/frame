INSERT INTO legacy_user_account_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'organization_projection', 1, COUNT(*)
FROM legacy_user_account_organization_ids_v1
WHERE organization_id = ?2
  AND legacy_organization_id = ?3;
