INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', 1,
  (SELECT COUNT(*) FROM legacy_developer_app_domains_v1 domain_row
   WHERE domain_row.id = ?2 AND domain_row.legacy_domain_id = ?3
     AND domain_row.app_id = ?4 AND domain_row.origin = ?5
     AND domain_row.last_operation_id = ?1)
)
