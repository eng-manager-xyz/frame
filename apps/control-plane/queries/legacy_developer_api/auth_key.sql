SELECT app.id AS app_id, app.environment
FROM legacy_developer_api_keys_v1 AS api_key
JOIN legacy_developer_apps_v1 AS app ON app.id = api_key.app_id
WHERE api_key.key_digest = ?1
  AND api_key.key_kind = ?2
  AND api_key.revoked_at_ms IS NULL
  AND app.deleted_at_ms IS NULL
LIMIT 1
