SELECT COUNT(*) AS target_count
FROM legacy_developer_videos_v1
WHERE id = ?1 AND app_id = ?2 AND deleted_at_ms IS NULL
