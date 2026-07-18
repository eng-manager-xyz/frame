INSERT INTO legacy_collaboration_receipts_v1(
  operation_id, result_kind, legacy_comment_id, legacy_video_id,
  legacy_author_id, author_name, author_image, comment_kind, content,
  source_timestamp, legacy_parent_comment_id, created_comment_at_ms,
  updated_comment_at_ms, notification_kind, deleted_comment_count,
  deleted_notification_count, notification_selector, revalidation_path,
  recorded_at_ms
)
SELECT
  ?1, 'created', comment.legacy_comment_id, comment.legacy_video_id,
  comment.legacy_author_id, ?2, ?3, comment.comment_kind, comment.content,
  comment.source_timestamp, comment.legacy_parent_comment_id,
  comment.created_at_ms, comment.updated_at_ms, comment.notification_kind,
  0, 0, NULL, ?4, ?5
FROM legacy_collaboration_comments_v1 comment
WHERE comment.last_operation_id = ?1;
