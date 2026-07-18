UPDATE storage_integrations SET state='revoked', credential_ciphertext=NULL,
  updated_at_ms=?3, revision=revision+1, authority_version=authority_version+1,
  last_operation_id=?4
WHERE id=?1 AND organization_id=?2
