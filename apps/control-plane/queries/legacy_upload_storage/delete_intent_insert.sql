INSERT INTO legacy_upload_storage_delete_intents_v1(
  operation_id, storage_integration_id, object_key, state, created_at_ms, completed_at_ms
) VALUES (?1, ?2, ?3, 'storage_pending', ?4, NULL);
