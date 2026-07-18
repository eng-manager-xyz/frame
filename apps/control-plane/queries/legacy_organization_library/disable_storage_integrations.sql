UPDATE storage_integrations
SET state = 'disabled', updated_at_ms = ?2, revision = revision + 1,
    authority_version = authority_version + 1, last_operation_id = ?3
WHERE organization_id = ?1 AND state = 'active'
