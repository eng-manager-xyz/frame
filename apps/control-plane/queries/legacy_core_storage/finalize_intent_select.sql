SELECT
  operation_id,
  state
FROM legacy_core_storage_finalize_intents_v1
WHERE mapped_video_id = ?1
  AND legacy_video_id = ?2
  AND actor_id = ?3
  AND organization_id = ?4
LIMIT 1;
