UPDATE videos
SET legacy_duration_seconds = COALESCE(?3, legacy_duration_seconds),
    duration_ms = COALESCE(?4, duration_ms),
    legacy_storage_width = COALESCE(?5, legacy_storage_width),
    legacy_storage_height = COALESCE(?6, legacy_storage_height),
    legacy_storage_fps = COALESCE(?7, legacy_storage_fps),
    updated_at_ms = MAX(updated_at_ms, ?8),
    revision = revision + 1
WHERE id = ?1
  AND owner_id = ?2
  AND deleted_at_ms IS NULL;
