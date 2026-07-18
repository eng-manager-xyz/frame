SELECT
  comment.legacy_comment_id AS legacy_comment_id,
  comment.legacy_video_id AS legacy_video_id,
  comment.comment_kind AS comment_kind,
  comment.content AS content,
  comment.source_timestamp AS source_timestamp,
  comment.legacy_parent_comment_id AS legacy_parent_comment_id,
  comment.created_at_ms AS created_at_ms,
  comment.updated_at_ms AS updated_at_ms,
  comment.legacy_author_id AS legacy_author_id,
  author.display_name AS author_name,
  COALESCE(author_alias.image_url, author.legacy_image_key) AS author_image
FROM legacy_collaboration_comments_v1 comment
LEFT JOIN users author ON author.id = comment.author_user_id
LEFT JOIN legacy_collaboration_user_aliases_v1 author_alias
  ON author_alias.mapped_user_id = comment.author_user_id
WHERE comment.legacy_video_id = ?1
ORDER BY comment.created_at_ms, comment.legacy_comment_id;
