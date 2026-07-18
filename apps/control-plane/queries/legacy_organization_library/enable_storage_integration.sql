UPDATE storage_integrations
SET state = 'active', updated_at_ms = ?3, revision = revision + 1,
    authority_version = authority_version + 1, last_operation_id = ?4
WHERE id = (
  SELECT id FROM storage_integrations
  WHERE organization_id = ?1 AND provider IN (SELECT value FROM json_each(?2))
    AND state <> 'revoked'
  ORDER BY updated_at_ms DESC, id
  LIMIT 1
)
