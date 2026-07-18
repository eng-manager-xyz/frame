INSERT INTO legacy_core_storage_finalize_intents_v1(
  mapped_video_id, legacy_video_id, operation_id, actor_id,
  organization_id, state, created_at_ms, terminal_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, 'provider_pending', ?6, NULL)
ON CONFLICT(mapped_video_id) DO NOTHING;
