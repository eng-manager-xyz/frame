INSERT INTO developer_credit_transactions(
  id, account_id, transaction_type, amount_microcredits,
  balance_after_microcredits, reference_type, reference_id, idempotency_key,
  metadata_json, created_at_ms, ledger_sequence, reference_digest,
  operation_id, request_fingerprint
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9,?10,?11,?12,?13)
