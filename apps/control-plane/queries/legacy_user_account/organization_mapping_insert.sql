INSERT INTO legacy_user_account_organization_ids_v1(
  organization_id, legacy_organization_id, recorded_at_ms, last_operation_id
) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(organization_id) DO UPDATE SET
  legacy_organization_id = excluded.legacy_organization_id,
  recorded_at_ms = excluded.recorded_at_ms,
  last_operation_id = excluded.last_operation_id
WHERE legacy_user_account_organization_ids_v1.legacy_organization_id
    = excluded.legacy_organization_id;
