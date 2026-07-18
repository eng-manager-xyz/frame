SELECT uploaded, total, updated_at_ms, mode, revision
FROM legacy_desktop_video_uploads_v1
WHERE video_id = ?1
LIMIT 2;
