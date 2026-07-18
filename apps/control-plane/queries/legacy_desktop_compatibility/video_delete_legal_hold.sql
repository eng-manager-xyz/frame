SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM legacy_desktop_video_delete_objects_v1 pending
  JOIN storage_objects object ON object.object_key = pending.object_key
  JOIN object_legal_holds hold ON hold.storage_object_id = object.id
  WHERE pending.operation_id = ?1
    AND pending.state = 'pending'
    AND hold.released_at_ms IS NULL
) THEN 1 ELSE 0 END AS has_legal_hold;
