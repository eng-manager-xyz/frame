SELECT delivery_id,
       identifier_candidates_json,
       active_identifier_key_version,
       active_identifier_digest,
       secret_key_version,
       secret_digest,
       purpose,
       channel,
       initiator_session_id,
       initiator_user_id,
       initiator_generation,
       provisioning_user_id,
       provisioning_revision,
       max_attempts,
       created_at_ms,
       expires_at_ms,
       sealed_payload_hex,
       revision
FROM auth_pending_verifications_v2
WHERE delivery_id = ?1
LIMIT 1
