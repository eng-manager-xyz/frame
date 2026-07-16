INSERT INTO usage_ledger(
  id, organization_id, app_id, video_id, media_job_id, usage_type, quantity,
  microcredits_charged, idempotency_key, occurred_at_ms, recorded_at_ms,
  metadata_json, operation_id, request_fingerprint
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13)
