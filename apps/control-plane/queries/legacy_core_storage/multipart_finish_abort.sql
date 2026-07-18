UPDATE legacy_core_storage_multipart_v1
SET state = 'aborted', terminal_at_ms = ?4
WHERE external_upload_id = ?1
  AND actor_id = ?2
  AND abort_operation_id = ?3
  AND state = 'abort_pending';
