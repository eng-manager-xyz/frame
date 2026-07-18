INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', 1,
  (SELECT COUNT(*) FROM legacy_developer_apps_v1 app
   WHERE app.id = ?2 AND app.owner_id = ?3 AND app.deleted_at_ms IS NULL
     AND app.name = ?4 AND app.environment = ?5 AND app.logo_url IS ?6
     AND app.revision = ?7 AND app.authority_version = ?8
     AND app.last_operation_id IS ?9)
)
