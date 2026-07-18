INSERT INTO storage_integrations(
  id, organization_id, owner_user_id, provider, state, capabilities_json,
  credential_ciphertext, created_at_ms, updated_at_ms, revision,
  authority_version, last_operation_id, capabilities_schema_version,
  capabilities_checksum
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
ON CONFLICT(id) DO UPDATE SET
  state = excluded.state,
  capabilities_json = excluded.capabilities_json,
  credential_ciphertext = excluded.credential_ciphertext,
  updated_at_ms = excluded.updated_at_ms,
  revision = excluded.revision,
  authority_version = excluded.authority_version,
  last_operation_id = excluded.last_operation_id,
  capabilities_schema_version = excluded.capabilities_schema_version,
  capabilities_checksum = excluded.capabilities_checksum
WHERE storage_integrations.organization_id = excluded.organization_id
  AND storage_integrations.owner_user_id IS excluded.owner_user_id
  AND storage_integrations.provider = excluded.provider
  AND storage_integrations.created_at_ms = excluded.created_at_ms
  AND storage_integrations.revision = ?15
  AND storage_integrations.authority_version = ?16
