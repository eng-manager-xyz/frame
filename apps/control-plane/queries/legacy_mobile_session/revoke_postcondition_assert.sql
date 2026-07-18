INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'bearer_absent_after_revoke', 0, COUNT(*)
FROM auth_api_keys
WHERE key_digest = ?2
