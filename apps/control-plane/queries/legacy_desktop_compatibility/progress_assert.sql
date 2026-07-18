INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1,
  CASE ?2
    WHEN 'deleted' THEN CASE WHEN NOT EXISTS (
      SELECT 1 FROM legacy_desktop_video_uploads_v1 WHERE video_id = ?3
    ) THEN 1 ELSE 0 END
    WHEN 'missing' THEN CASE WHEN NOT EXISTS (
      SELECT 1 FROM legacy_desktop_video_uploads_v1 WHERE video_id = ?3
    ) THEN 1 ELSE 0 END
    WHEN 'stale' THEN CASE WHEN EXISTS (
      SELECT 1 FROM legacy_desktop_video_uploads_v1
      WHERE video_id = ?3 AND revision = ?4 AND updated_at_ms = ?5
    ) THEN 1 ELSE 0 END
    ELSE CASE WHEN EXISTS (
      SELECT 1 FROM legacy_desktop_video_uploads_v1
      WHERE video_id = ?3 AND uploaded = ?6 AND total = ?7
        AND updated_at_ms = ?8 AND last_operation_id = ?1
    ) THEN 1 ELSE 0 END
  END;
