INSERT INTO legacy_collaboration_receipts_v1(
  operation_id, result_kind, legacy_comment_id, legacy_video_id,
  legacy_author_id, author_name, author_image, comment_kind, content,
  source_timestamp, legacy_parent_comment_id, created_comment_at_ms,
  updated_comment_at_ms, notification_kind, deleted_comment_count,
  deleted_notification_count, notification_selector, revalidation_path,
  recorded_at_ms
)
SELECT
  ?1, 'deleted', ?2, NULL,
  NULL, NULL, NULL, NULL, NULL,
  NULL, NULL, NULL,
  NULL, NULL,
  (SELECT COUNT(*) FROM legacy_collaboration_delete_targets_v1 target
    WHERE target.operation_id = ?1),
  (SELECT COUNT(*) FROM legacy_collaboration_notification_targets_v1 target
    WHERE target.operation_id = ?1),
  ?3, ?4, ?5;
