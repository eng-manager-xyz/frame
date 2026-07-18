INSERT INTO legacy_collaboration_comments_v1(
  legacy_comment_id, mapped_comment_id, legacy_video_id, mapped_video_id,
  author_user_id, legacy_author_id, comment_kind, content, source_timestamp,
  legacy_parent_comment_id, notification_kind, created_at_ms, updated_at_ms,
  source_action, last_operation_id
)
VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14
);
