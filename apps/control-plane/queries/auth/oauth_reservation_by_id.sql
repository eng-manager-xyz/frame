SELECT id,
       flow_id,
       provider,
       initiator_session_id,
       initiator_user_id,
       initiator_generation,
       expires_at_ms,
       created_at_ms,
       consumed_at_ms,
       revision,
       last_operation_id
FROM auth_oauth_reservations_v2
WHERE id = ?1
LIMIT 1
