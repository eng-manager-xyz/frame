INSERT OR IGNORE INTO object_deletion_jobs(
  id, storage_object_id, idempotency_key, state, not_before_ms,
  attempt, error_class, created_at_ms, updated_at_ms
)
SELECT
  ?1 || ':' || object.id,
  object.id,
  'legacy-desktop-video-delete:' || ?1 || ':' || object.id,
  CASE WHEN EXISTS (
    SELECT 1 FROM object_legal_holds hold
    WHERE hold.storage_object_id = object.id AND hold.released_at_ms IS NULL
  ) THEN 'blocked_by_hold' ELSE 'scheduled' END,
  ?3, 0, NULL, ?3, ?3
FROM storage_objects object
WHERE object.video_id = ?2 AND object.state NOT IN ('deleted','missing');
