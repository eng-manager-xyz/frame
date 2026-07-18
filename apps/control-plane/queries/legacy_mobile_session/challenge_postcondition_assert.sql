INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'challenge_and_handoff', 1, COUNT(*)
FROM legacy_mobile_session_challenges_v1 challenge
JOIN auth_delivery_provider_handoffs_v1 handoff
  ON handoff.delivery_id = challenge.delivery_id
WHERE challenge.identifier_digest = ?2
  AND challenge.token_digest = ?3
  AND challenge.delivery_id = ?4
  AND challenge.request_operation_id = ?1
  AND challenge.expires_at_ms = ?5 + 600000
  AND handoff.state = 'pending'
