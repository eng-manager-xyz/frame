INSERT OR IGNORE INTO legacy_desktop_video_delete_objects_v1(
  operation_id, object_key, state, deleted_at_ms
)
SELECT ?1, object_key, 'pending', NULL
FROM storage_objects
WHERE video_id = ?2 AND state NOT IN ('deleted','missing')
UNION
SELECT ?1, object_key, 'pending', NULL
FROM object_manifests
WHERE video_id = ?2 AND state NOT IN ('deleted','missing');
