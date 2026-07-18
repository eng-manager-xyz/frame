INSERT INTO legacy_protected_media_execution_outbox_v1 (
  receipt_id,
  executor_kind,
  descriptor_json,
  descriptor_digest,
  state,
  attempt_count,
  created_at_ms
) VALUES (
  ?1, ?2, ?3, ?4, 'pending_execution_evidence', 0, ?5
);
