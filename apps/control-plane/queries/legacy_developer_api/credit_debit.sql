INSERT INTO legacy_developer_credit_transactions_v1(
  id, account_id, transaction_type, amount_microcredits, balance_after_microcredits,
  reference_id, reference_type, metadata_json, operation_id, created_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
