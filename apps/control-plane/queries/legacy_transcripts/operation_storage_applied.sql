UPDATE legacy_transcript_operations_v1
SET state = 'storage_applied',
    attempt_count = attempt_count + 1,
    updated_at_ms = CASE WHEN updated_at_ms < ?2 THEN ?2 ELSE updated_at_ms END
WHERE operation_id = ?1
  AND request_digest = ?3
  AND state IN ('claimed', 'storage_applied');
