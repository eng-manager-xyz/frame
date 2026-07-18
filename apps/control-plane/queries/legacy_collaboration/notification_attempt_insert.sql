INSERT OR IGNORE INTO legacy_collaboration_notification_attempts_v1(
  operation_id, legacy_comment_id, notification_kind, outcome, attempted_at_ms
)
VALUES (?1, ?2, ?3, 'handoff_queued', ?4);
