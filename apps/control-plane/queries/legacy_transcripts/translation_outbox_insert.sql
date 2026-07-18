INSERT OR IGNORE INTO legacy_transcript_translation_outbox_v1(
  operation_id, model, source_object_key, target_object_key, target_language,
  prompt_contract_version, state, created_at_ms, updated_at_ms
) VALUES(?1,'openai/gpt-oss-120b',?2,?3,?4,'cap.translate-vtt.v1','pending',?5,?5);
