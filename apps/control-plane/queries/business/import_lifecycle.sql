SELECT state, event_sequence, event_fingerprint, revision
FROM imported_videos
WHERE id = ?1 AND organization_id = ?2
LIMIT 2
