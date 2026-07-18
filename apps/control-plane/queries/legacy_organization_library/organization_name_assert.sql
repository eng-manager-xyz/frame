INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'organization_name_available', 0,
  (
    SELECT COUNT(*) FROM organizations
    WHERE status <> 'deleted'
      AND COALESCE(legacy_user_account_name, name) = ?2
  )
)
