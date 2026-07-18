INSERT INTO legacy_developer_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'postcondition', ?4,
  (SELECT COUNT(*) FROM legacy_developer_videos_v1 video
   WHERE video.id = ?2 AND video.app_id = ?3 AND video.deleted_at_ms = ?5
     AND video.last_operation_id = ?1)
)
