INSERT INTO legacy_developer_credit_accounts_v1(
  id, legacy_credit_account_id, app_id, owner_id, balance_microcredits,
  auto_top_up_enabled, auto_top_up_threshold_microcredits,
  auto_top_up_amount_cents, created_at_ms, updated_at_ms, revision,
  last_operation_id
)
VALUES (?1, ?2, ?3, ?4, 0, 0, 0, 0, ?5, ?5, 0, ?6)
