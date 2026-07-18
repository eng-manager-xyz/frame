INSERT INTO legacy_developer_app_domains_v1(
  id, legacy_domain_id, app_id, origin, created_at_ms, revision, last_operation_id
)
SELECT ?1, ?2, ?3, ?4, ?5, 0, ?6
WHERE EXISTS (
  SELECT 1 FROM legacy_developer_apps_v1 app
  WHERE app.id = ?3 AND app.owner_id = ?7 AND app.deleted_at_ms IS NULL
    AND app.revision = ?8 AND app.authority_version = ?9
)
