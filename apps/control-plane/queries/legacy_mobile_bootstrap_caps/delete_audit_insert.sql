INSERT INTO legacy_mobile_cap_delete_audit_v1(
  audit_id, operation_id, actor_digest, video_digest, outcome, occurred_at_ms
) VALUES (?1, ?2, ?3, ?4, 'authorized_storage_pending', ?5);
