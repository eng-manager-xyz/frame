INSERT INTO legacy_mobile_cap_uploads_v1(
  mapped_video_id, uploaded, total, phase, processing_progress,
  processing_message, processing_error, raw_file_key, updated_at_ms, started_at_ms
) VALUES (?1, 0, 0, 'uploading', 0, NULL, NULL, NULL, ?2, ?2);
