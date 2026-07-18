UPDATE legacy_core_storage_multipart_v1
SET state = 'abort_pending',
    abort_operation_id = ?3
WHERE external_upload_id = ?1
  AND actor_id = ?2
  AND state = 'open'
  AND expires_at_ms > ?4;
