UPDATE legacy_extension_instant_recordings_v1
SET lifecycle_state = 'deleted',
    storage_cleanup_state = 'complete',
    deleted_at_ms = ?3,
    last_operation_id = ?4
WHERE legacy_video_id = ?1
  AND mapped_video_id = ?2
  AND lifecycle_state = 'deleting'
  AND storage_cleanup_state = 'pending';
