INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', ?3,
  (SELECT COUNT(*) FROM legacy_developer_credit_accounts_v1 credit
   WHERE credit.app_id = ?2 AND credit.owner_id = ?4
     AND credit.auto_top_up_enabled = ?5
     AND credit.auto_top_up_threshold_microcredits = ?6
     AND credit.auto_top_up_amount_cents = ?7
     AND credit.revision = ?8 AND credit.last_operation_id = ?1)
)
