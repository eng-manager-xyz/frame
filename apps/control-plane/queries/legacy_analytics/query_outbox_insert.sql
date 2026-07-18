INSERT INTO legacy_analytics_query_outbox_v1(
  operation_id, query_kind, request_json, request_digest, created_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5)
