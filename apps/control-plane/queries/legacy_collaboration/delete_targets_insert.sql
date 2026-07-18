INSERT INTO legacy_collaboration_delete_targets_v1(
  operation_id, legacy_comment_id, target_role, ordinal
)
SELECT ?1, candidate.legacy_comment_id, candidate.target_role,
       ROW_NUMBER() OVER (
         ORDER BY CASE candidate.target_role WHEN 'target' THEN 0 ELSE 1 END,
                  candidate.legacy_comment_id
       ) - 1
FROM (
  SELECT
    comment.legacy_comment_id,
    CASE WHEN comment.legacy_comment_id = ?3
      THEN 'target' ELSE 'authored_direct_reply' END AS target_role
  FROM legacy_collaboration_comments_v1 comment
  WHERE comment.author_user_id = ?2
    AND (
      comment.legacy_comment_id = ?3
      OR (?4 = 'route' AND comment.legacy_parent_comment_id = ?3)
    )
  ORDER BY CASE WHEN comment.legacy_comment_id = ?3 THEN 0 ELSE 1 END,
           comment.legacy_comment_id
  LIMIT 100001
) candidate;
