INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', 1,
  (SELECT COUNT(*) FROM legacy_developer_apps_v1 app
   JOIN legacy_developer_credit_accounts_v1 credit
     ON credit.app_id = app.id AND credit.owner_id = app.owner_id
   WHERE app.id = ?2 AND app.legacy_app_id = ?3 AND app.owner_id = ?4
     AND app.name = ?5 AND app.environment = ?6 AND app.deleted_at_ms IS NULL
     AND credit.id = ?7 AND credit.balance_microcredits = 0
     AND credit.auto_top_up_enabled = 0
     AND credit.auto_top_up_threshold_microcredits = 0
     AND credit.auto_top_up_amount_cents = 0
     AND (SELECT COUNT(*) FROM legacy_developer_api_keys_v1 key_row
          WHERE key_row.app_id = app.id AND key_row.revoked_at_ms IS NULL) = 2)
)
