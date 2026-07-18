UPDATE legacy_desktop_personal_storage_integrations_v1
SET active = 1,
    updated_at_ms = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE integration_id = ?1
  AND owner_user_id = ?2
  AND provider = 'googleDrive'
  AND status = 'active';
