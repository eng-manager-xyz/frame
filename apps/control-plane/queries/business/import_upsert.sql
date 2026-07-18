INSERT INTO imported_videos(
  id, organization_id, video_id, provider, external_id_digest, state,
  idempotency_key, error_class, created_at_ms, updated_at_ms,
  event_sequence, event_fingerprint, revision, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
ON CONFLICT(id) DO NOTHING
