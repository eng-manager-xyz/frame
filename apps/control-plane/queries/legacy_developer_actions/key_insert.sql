INSERT INTO legacy_developer_api_keys_v1(
  id, legacy_key_id, app_id, key_kind, key_prefix, key_digest,
  encrypted_key, revoked_at_ms, created_at_ms, revision, last_operation_id
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 0, ?9)
