INSERT INTO authenticated_web_action_assertions_v1(
  operation_id,assertion_kind,expected_count,actual_count
)
VALUES(
  ?1,
  'operation_complete',
  1,
  (
    SELECT COUNT(*)
    FROM legacy_protected_billing_auth_receipts_v1 receipt
    JOIN legacy_protected_billing_auth_live_authority_v1 live
      ON live.receipt_id = receipt.receipt_id
    WHERE receipt.source_operation_id = ?2
      AND receipt.principal_digest = ?3
      AND receipt.replay_key_digest = ?4
      AND receipt.request_digest = ?5
      AND live.authority_expires_at_ms > ?6
  )
);
