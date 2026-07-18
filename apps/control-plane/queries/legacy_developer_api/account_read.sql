SELECT account.id, account.balance_microcredits, app.legacy_app_id
FROM legacy_developer_credit_accounts_v1 AS account
JOIN legacy_developer_apps_v1 AS app ON app.id = account.app_id
WHERE account.app_id = ?1 LIMIT 1
