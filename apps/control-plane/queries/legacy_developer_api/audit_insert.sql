INSERT INTO legacy_developer_api_audit_v1(
  id, operation_id, source_operation_id, app_digest, target_digest,
  request_digest, result_digest, occurred_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
