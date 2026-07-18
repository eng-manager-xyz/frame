INSERT INTO legacy_protected_integration_generated_replay_claims_v1(
  source_operation_id,principal_digest,request_digest,receipt_id,claimed_at_ms
) VALUES(?1,?2,?3,?4,?5)
ON CONFLICT(source_operation_id,principal_digest,request_digest) DO UPDATE SET
  receipt_id=excluded.receipt_id,
  claimed_at_ms=excluded.claimed_at_ms
WHERE EXISTS (
  SELECT 1 FROM legacy_protected_integration_receipts_v1 prior
  WHERE prior.receipt_id = legacy_protected_integration_generated_replay_claims_v1.receipt_id
    AND prior.state IN ('verified','dead_letter')
    AND prior.completed_at_ms IS NOT NULL
    AND prior.completed_at_ms <= ?6
);
