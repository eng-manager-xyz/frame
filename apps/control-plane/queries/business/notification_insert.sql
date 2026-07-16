INSERT INTO notifications(
  id, organization_id, recipient_user_id, type, deduplication_key, data_json,
  created_at_ms, read_at_ms, payload_schema_version, payload_checksum, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
