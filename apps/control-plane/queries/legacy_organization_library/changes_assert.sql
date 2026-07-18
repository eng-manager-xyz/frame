INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (?1, ?2, ?3, changes())
