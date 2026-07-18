INSERT INTO auth_external_accounts_v2(
  provider, subject_key_version, subject_digest, user_id,
  created_at_ms, updated_at_ms, revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0, ?6)
ON CONFLICT(provider, subject_key_version, subject_digest) DO UPDATE SET
  updated_at_ms = excluded.updated_at_ms,
  revision = auth_external_accounts_v2.revision + 1,
  last_operation_id = excluded.last_operation_id
WHERE auth_external_accounts_v2.user_id = excluded.user_id
