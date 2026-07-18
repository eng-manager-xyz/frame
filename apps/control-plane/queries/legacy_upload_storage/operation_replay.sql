SELECT operation_id, mapped_video_id, legacy_video_id, request_digest, state, result_json
FROM legacy_upload_storage_operations_v1
WHERE source_operation_id = ?1 AND actor_id = ?2 AND idempotency_key_digest = ?3
LIMIT 2;
