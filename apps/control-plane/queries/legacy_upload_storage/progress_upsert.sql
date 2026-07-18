INSERT INTO legacy_mobile_cap_uploads_v1(
  mapped_video_id, uploaded, total, phase, processing_progress,
  processing_message, processing_error, raw_file_key, updated_at_ms, started_at_ms
) VALUES (?1, ?2, ?3, 'uploading', 0, NULL, NULL, NULL, ?4, ?4)
ON CONFLICT(mapped_video_id) DO UPDATE SET
  uploaded = excluded.uploaded,
  total = excluded.total,
  updated_at_ms = excluded.updated_at_ms
WHERE legacy_mobile_cap_uploads_v1.updated_at_ms <= excluded.updated_at_ms;
