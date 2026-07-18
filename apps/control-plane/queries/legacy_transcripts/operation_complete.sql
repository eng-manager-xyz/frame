UPDATE legacy_transcript_operations_v1
SET state = 'complete', result_json = ?2, failure_code = NULL,
    updated_at_ms = CASE WHEN updated_at_ms < ?3 THEN ?3 ELSE updated_at_ms END,
    completed_at_ms = ?3
WHERE operation_id = ?1
  AND request_digest = ?4
  AND state IN ('claimed', 'storage_applied');
