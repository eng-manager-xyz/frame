SELECT id, video_id, parent_comment_id, author_user_id, anonymous_author_digest,
       body, comment_kind, timeline_micros, created_at_ms, updated_at_ms,
       deleted_at_ms, revision
FROM comments
WHERE organization_id=?1 AND video_id=?2 AND deleted_at_ms IS NULL
ORDER BY created_at_ms, id
LIMIT 1000
