UPDATE object_manifests
SET state = 'deleting', updated_at_ms = ?3
WHERE video_id = ?2 AND state NOT IN ('deleted','missing');
