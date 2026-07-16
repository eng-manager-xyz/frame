INSERT INTO video_uploads(
  id, organization_id, video_id, state, expected_bytes, received_bytes,
  idempotency_key, source_object_key, source_version, content_type,
  checksum_sha256, created_at_ms, updated_at_ms, revision, event_sequence,
  event_fingerprint, last_operation_id
) VALUES (
  ?1,?2,?3,'initiated',?4,0,?5,?6,?7,?8,NULL,?9,?9,0,0,
  'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9',?10
)
