INSERT INTO legacy_collaboration_notification_targets_v1(
  operation_id, notification_id, notification_type
)
SELECT ?1, notification.id, notification.type
FROM notifications notification
WHERE
  (
    ?2 = 'reply_by_comment_id'
    AND notification.type = 'reply'
    AND json_extract(notification.data_json, '$.comment.id') = ?3
  )
  OR (
    ?2 = 'root_comment_and_replies_by_parent_id'
    AND (
      (notification.type = 'comment'
        AND json_extract(notification.data_json, '$.comment.id') = ?3)
      OR (notification.type = 'reply'
        AND json_extract(notification.data_json, '$.comment.parentCommentId') = ?3)
    )
  )
ORDER BY notification.id
LIMIT 100001;
