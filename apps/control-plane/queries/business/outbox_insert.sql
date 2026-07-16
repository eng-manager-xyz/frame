INSERT INTO outbox_events(
  id, organization_id, aggregate_type, aggregate_id, event_type,
  deduplication_key, payload_json, state, attempt, available_at_ms,
  lease_expires_at_ms, created_at_ms, delivered_at_ms, event_sequence,
  event_fingerprint, payload_schema_version, payload_checksum, revision,
  last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,?9,NULL,?10,NULL,?11,?12,?13,?14,?15,?16)
