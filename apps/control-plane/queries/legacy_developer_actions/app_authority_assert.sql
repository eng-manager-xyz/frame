INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'app_authority', 1,
  (SELECT COUNT(*) FROM legacy_developer_apps_v1 app
   WHERE app.id = ?2 AND app.owner_id = ?3 AND app.deleted_at_ms IS NULL
     AND app.revision = ?4 AND app.authority_version = ?5)
)
