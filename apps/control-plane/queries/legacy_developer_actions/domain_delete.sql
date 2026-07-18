DELETE FROM legacy_developer_app_domains_v1
WHERE id = ?2 AND app_id = ?3
  AND EXISTS (
    SELECT 1 FROM legacy_developer_apps_v1 app
    WHERE app.id = ?3 AND app.owner_id = ?4 AND app.deleted_at_ms IS NULL
      AND app.revision = ?5 AND app.authority_version = ?6
  )
