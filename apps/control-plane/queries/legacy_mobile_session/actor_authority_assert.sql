INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'actor_authority', 1, COUNT(*)
FROM users u
JOIN legacy_collaboration_user_aliases_v1 a ON a.mapped_user_id = u.id
WHERE u.id = ?2 AND a.legacy_user_id = ?3
