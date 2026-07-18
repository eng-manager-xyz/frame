INSERT INTO legacy_user_account_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'organization_access', 1, COUNT(*)
FROM organizations o
WHERE o.id = ?2
  AND (
    o.owner_id = ?3
    OR EXISTS (
      SELECT 1 FROM organization_members m
      WHERE m.organization_id = o.id AND m.user_id = ?3
    )
  );
