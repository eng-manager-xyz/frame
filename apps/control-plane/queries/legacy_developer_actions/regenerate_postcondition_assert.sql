INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', 2,
  (SELECT COUNT(*) FROM legacy_developer_api_keys_v1 key_row
   WHERE key_row.app_id = ?2 AND key_row.revoked_at_ms IS NULL
     AND key_row.last_operation_id = ?1
     AND key_row.key_kind IN ('public', 'secret'))
)
