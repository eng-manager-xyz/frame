UPDATE legacy_developer_api_keys_v1
SET revoked_at_ms = ?4, revision = revision + 1, last_operation_id = ?1
WHERE app_id = ?2 AND revoked_at_ms IS NULL
  AND EXISTS (
    SELECT 1 FROM legacy_developer_apps_v1 app
    WHERE app.id = ?2 AND app.owner_id = ?3 AND app.deleted_at_ms IS NULL
  )
