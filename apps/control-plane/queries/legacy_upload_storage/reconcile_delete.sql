DELETE FROM legacy_mobile_cap_uploads_v1
WHERE mapped_video_id = ?1
  AND raw_file_key = ?2
  AND updated_at_ms = ?3
  AND phase = ?4
  AND processing_progress = ?5;
