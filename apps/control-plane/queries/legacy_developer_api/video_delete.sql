UPDATE legacy_developer_videos_v1
SET deleted_at_ms = ?3, updated_at_ms = ?3, revision = revision + 1,
    last_operation_id = ?4
WHERE app_id = ?1 AND legacy_video_id = ?2 AND deleted_at_ms IS NULL
