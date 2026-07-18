SELECT
  operation_id, request_digest, state, result_json, failure_code,
  object_key, target_language, entry_id, replacement_text, attempt_count
FROM legacy_transcript_operations_v1
WHERE source_operation_id = ?1
  AND actor_scope_digest = ?2
  AND mapped_video_id = ?3
  AND idempotency_key_digest = ?4
LIMIT 2;
