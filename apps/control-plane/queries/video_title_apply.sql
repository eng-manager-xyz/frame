INSERT INTO repository_video_title_operations(
  operation_id,
  organization_id,
  video_id,
  actor_id,
  idempotency_key,
  request_digest,
  reservation_id,
  outbox_id,
  deduplication_key,
  expected_revision,
  title,
  response_json,
  payload_json,
  now_ms,
  expires_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
