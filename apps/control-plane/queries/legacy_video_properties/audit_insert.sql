INSERT INTO legacy_video_property_audit_v1 (
  audit_id, operation_id, source_operation_id, principal_digest,
  video_id_digest, request_digest, result_digest, occurred_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);
