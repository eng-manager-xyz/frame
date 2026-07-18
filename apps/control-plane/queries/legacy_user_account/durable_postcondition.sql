INSERT INTO legacy_user_account_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'durable_postcondition', 4,
  (SELECT COUNT(*) FROM legacy_user_account_operations_v1
    WHERE operation_id = ?1 AND state = 'applied')
  + (SELECT COUNT(*) FROM legacy_user_account_receipts_v1 WHERE operation_id = ?1)
  + (SELECT COUNT(*) FROM legacy_user_account_effects_v1 WHERE operation_id = ?1)
  + (SELECT COUNT(*) FROM legacy_user_account_audit_events_v1 WHERE operation_id = ?1);
