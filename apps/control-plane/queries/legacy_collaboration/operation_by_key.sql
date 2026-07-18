SELECT
  operation.operation_id,
  operation.organization_id,
  operation.actor_id,
  operation.action,
  operation.request_digest,
  operation.state,
  receipt.result_kind,
  receipt.legacy_comment_id,
  receipt.legacy_video_id,
  receipt.legacy_author_id,
  receipt.author_name,
  receipt.author_image,
  receipt.comment_kind,
  receipt.content,
  receipt.source_timestamp,
  receipt.legacy_parent_comment_id,
  receipt.created_comment_at_ms,
  receipt.updated_comment_at_ms,
  receipt.notification_kind,
  receipt.deleted_comment_count,
  receipt.deleted_notification_count,
  receipt.notification_selector,
  receipt.revalidation_path,
  effect.notification_timing,
  effect.notification_failure_rolls_back_core,
  effect.revalidation_path AS effect_revalidation_path,
  (SELECT COUNT(*) FROM legacy_collaboration_audit_events_v1 audit
    WHERE audit.operation_id = operation.operation_id) AS audit_count
FROM legacy_collaboration_operations_v1 operation
LEFT JOIN legacy_collaboration_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
LEFT JOIN legacy_collaboration_effects_v1 effect
  ON effect.operation_id = operation.operation_id
WHERE operation.organization_id = ?1
  AND operation.actor_id = ?2
  AND operation.action = ?3
  AND operation.idempotency_key_digest = ?4
LIMIT 2;
