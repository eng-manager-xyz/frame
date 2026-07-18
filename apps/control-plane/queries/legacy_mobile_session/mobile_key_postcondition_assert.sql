INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'single_mobile_key', 1, COUNT(*)
FROM auth_api_keys
WHERE id = ?2 AND user_id = ?3 AND key_digest = ?4
  AND legacy_source = 'mobile'
  AND (SELECT COUNT(*) FROM auth_api_keys
       WHERE user_id = ?3 AND legacy_source = 'mobile') = 1
