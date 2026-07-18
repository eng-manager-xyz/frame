INSERT OR IGNORE INTO legacy_transcript_operations_v1(
  operation_id, source_operation_id, operation_kind, actor_scope_digest,
  actor_id, mapped_video_id, legacy_video_id, idempotency_key_digest,
  request_digest, object_key, target_language, entry_id, replacement_text,
  state, created_at_ms, updated_at_ms
) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15);
