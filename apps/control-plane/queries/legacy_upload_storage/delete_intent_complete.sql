UPDATE legacy_upload_storage_delete_intents_v1
SET state = 'complete', completed_at_ms = ?2
WHERE operation_id = ?1 AND state = 'storage_pending';
