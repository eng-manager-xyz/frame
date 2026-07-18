INSERT INTO legacy_upload_storage_capability_intents_v1(
  operation_id, storage_integration_id, object_key, method,
  content_type, expires_at_ms, created_at_ms
) VALUES (?1, ?2, ?3, 'PUT', ?4, ?5, ?6);
