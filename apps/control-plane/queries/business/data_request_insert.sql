INSERT INTO business_data_requests_v1(
  id, organization_id, data_class, subject_id, request_kind, disposition,
  manifest_checksum, requested_at_ms, completed_at_ms, operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
