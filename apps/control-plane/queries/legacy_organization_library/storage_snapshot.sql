SELECT
  integration.id,
  integration.provider,
  integration.state,
  integration.capabilities_json,
  integration.credential_ciphertext,
  integration.revision,
  integration.authority_version
FROM storage_integrations integration
WHERE integration.organization_id = ?1
ORDER BY
  CASE WHEN integration.state = 'active' THEN 0 ELSE 1 END,
  integration.updated_at_ms DESC,
  integration.id
LIMIT 100
