INSERT INTO legacy_user_account_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1, COUNT(*)
FROM users
WHERE id = ?2
  AND (
    (?3 = 0 AND legacy_user_account_revision = ?4)
    OR (?3 = 1 AND legacy_user_account_revision = ?4 + 1
      AND legacy_user_account_last_operation_id = ?1)
  );
