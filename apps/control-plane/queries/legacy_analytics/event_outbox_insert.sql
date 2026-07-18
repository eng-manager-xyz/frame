INSERT INTO legacy_analytics_event_outbox_v1(
  operation_id, timestamp_iso, session_id, tenant_id, video_id, pathname,
  country, region, city, browser, device, operating_system, raw_user_agent,
  user_id, event_json, created_at_ms
) VALUES(
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
)
