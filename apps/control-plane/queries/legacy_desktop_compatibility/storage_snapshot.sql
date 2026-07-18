SELECT integration_id, active, updated_at_ms, revision
FROM legacy_desktop_personal_storage_integrations_v1
WHERE owner_user_id = ?1
  AND provider = 'googleDrive'
  AND status = 'active'
ORDER BY active DESC, updated_at_ms DESC, integration_id
LIMIT 2;
