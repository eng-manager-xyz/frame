UPDATE legacy_developer_videos_v1
SET duration = ?2, width = ?3, height = ?4, fps = ?5,
    updated_at_ms = ?6, revision = revision + 1, last_operation_id = ?7
WHERE id = ?1 AND deleted_at_ms IS NULL
