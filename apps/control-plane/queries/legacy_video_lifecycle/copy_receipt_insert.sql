INSERT INTO legacy_video_lifecycle_copy_receipts_v1(
  operation_id, source_key, destination_key, source_version,
  source_bytes, copied_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(operation_id, source_key) DO NOTHING;
