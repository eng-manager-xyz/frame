SELECT state, event_sequence, event_fingerprint, revision
FROM video_uploads
WHERE id=?1 AND organization_id=?2
