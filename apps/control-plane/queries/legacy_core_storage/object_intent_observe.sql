UPDATE legacy_core_storage_object_intents_v1
SET state = 'observed', observed_at_ms = ?3
WHERE object_key = ?1
  AND mapped_video_id = ?2
  AND state = 'capability_issued';
