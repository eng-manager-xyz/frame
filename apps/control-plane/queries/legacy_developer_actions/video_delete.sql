UPDATE legacy_developer_videos_v1
SET deleted_at_ms = ?5, updated_at_ms = ?5, revision = revision + 1,
    last_operation_id = ?1
WHERE id = ?2 AND app_id = ?3 AND deleted_at_ms IS NULL
  AND EXISTS (
    SELECT 1 FROM legacy_developer_apps_v1 app
    WHERE app.id = ?3 AND app.owner_id = ?4 AND app.deleted_at_ms IS NULL
      AND app.revision = ?6 AND app.authority_version = ?7
  )
