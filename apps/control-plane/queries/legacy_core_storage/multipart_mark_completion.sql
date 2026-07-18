UPDATE legacy_core_storage_multipart_v1
SET state = 'completion_pending',
    completion_operation_id = ?3,
    expected_bytes = ?4,
    parts_digest = ?5
WHERE external_upload_id = ?1
  AND actor_id = ?2
  AND state = 'open'
  AND expires_at_ms > ?6;
