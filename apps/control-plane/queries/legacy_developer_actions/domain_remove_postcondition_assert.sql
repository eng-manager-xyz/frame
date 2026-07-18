INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', 0,
  (SELECT COUNT(*) FROM legacy_developer_app_domains_v1
   WHERE id = ?2 AND app_id = ?3)
)
