UPDATE legacy_extension_instant_recordings_v1
SET lifecycle_state = 'deleting',
    storage_cleanup_state = 'pending',
    delete_started_at_ms = ?3,
    last_operation_id = ?4
WHERE legacy_video_id = ?1
  AND mapped_video_id = ?2
  AND lifecycle_state = 'active';
