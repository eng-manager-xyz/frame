DELETE FROM legacy_collaboration_comments_v1
WHERE legacy_comment_id IN (
  SELECT legacy_comment_id
  FROM legacy_collaboration_delete_targets_v1
  WHERE operation_id = ?1
);
