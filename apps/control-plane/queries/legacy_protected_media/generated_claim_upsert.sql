INSERT INTO legacy_protected_media_generated_replay_claims_v1 (
  source_operation_id,principal_digest,request_digest,receipt_id,claimed_at_ms
) VALUES (?1,?2,?3,?4,?5)
ON CONFLICT(source_operation_id,principal_digest,request_digest) DO UPDATE SET
  receipt_id=excluded.receipt_id,
  claimed_at_ms=excluded.claimed_at_ms;
