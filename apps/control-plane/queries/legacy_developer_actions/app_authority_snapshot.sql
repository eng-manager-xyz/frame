SELECT
  app.id,
  app.legacy_app_id,
  app.owner_id,
  app.name,
  app.environment,
  app.logo_url,
  app.last_operation_id,
  app.revision,
  app.authority_version,
  (SELECT COUNT(*) FROM legacy_developer_api_keys_v1 key_row
    WHERE key_row.app_id = app.id AND key_row.revoked_at_ms IS NULL) AS active_key_count,
  credit.id AS credit_account_id,
  credit.legacy_credit_account_id,
  credit.auto_top_up_enabled,
  credit.auto_top_up_threshold_microcredits,
  credit.auto_top_up_amount_cents,
  credit.revision AS credit_revision
FROM legacy_developer_apps_v1 app
LEFT JOIN legacy_developer_credit_accounts_v1 credit ON credit.app_id = app.id
WHERE app.id = ?1 AND app.owner_id = ?2 AND app.deleted_at_ms IS NULL
LIMIT 2
