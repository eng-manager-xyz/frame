INSERT OR IGNORE INTO legacy_analytics_email_outbox_v1(
  operation_id, video_id, recipient_user_id, recipient_email,
  viewer_user_id, viewer_name, is_anonymous, payload_json, created_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
