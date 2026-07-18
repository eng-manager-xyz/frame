INSERT INTO legacy_user_account_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'authority', 1, COUNT(*)
FROM users
WHERE id = ?2
  AND status = 'active'
  AND deleted_at_ms IS NULL
  AND legacy_user_account_revision = ?3
  AND legacy_user_account_authority_version = ?4;
