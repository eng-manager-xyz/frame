UPDATE legacy_developer_multipart_sessions_v1
SET state = ?2, updated_at_ms = ?3,
    completed_at_ms = CASE WHEN ?2 IN ('complete','aborted') THEN ?3 ELSE completed_at_ms END,
    terminal_operation_id = COALESCE(terminal_operation_id, ?5),
    revision = revision + 1
WHERE provider_upload_id = ?1 AND state = ?4
  AND (terminal_operation_id IS NULL OR terminal_operation_id = ?5)
