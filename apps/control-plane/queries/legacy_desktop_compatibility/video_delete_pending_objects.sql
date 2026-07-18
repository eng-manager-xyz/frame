SELECT pending.object_key,
  CASE WHEN EXISTS (
    SELECT 1
    FROM storage_objects object
    JOIN object_legal_holds hold ON hold.storage_object_id = object.id
    WHERE object.object_key = pending.object_key AND hold.released_at_ms IS NULL
  ) THEN 1 ELSE 0 END AS has_legal_hold
FROM legacy_desktop_video_delete_objects_v1 pending
WHERE pending.operation_id = ?1 AND pending.state = 'pending'
ORDER BY pending.object_key
LIMIT 1000;
