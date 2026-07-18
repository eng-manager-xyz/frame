INSERT INTO legacy_mobile_upload_processing_intents_v1(
  mapped_video_id, operation_id, actor_id, organization_id, raw_file_key,
  observed_bytes, requested_content_length, state, created_at_ms,
  submitted_at_ms, terminal_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'provider_pending', ?8, NULL, NULL);
