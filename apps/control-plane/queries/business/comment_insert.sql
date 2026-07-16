INSERT INTO comments(
  id, video_id, parent_comment_id, author_user_id, anonymous_author_digest,
  body, created_at_ms, updated_at_ms, deleted_at_ms, revision,
  organization_id, last_operation_id, comment_kind, timeline_micros
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
