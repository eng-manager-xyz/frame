INSERT INTO legacy_developer_provider_outbox_v1(
  operation_id, effect_kind, payload_digest, state, attempt_count, created_at_ms
) VALUES(?1, ?2, ?3, 'pending', 0, ?4)
