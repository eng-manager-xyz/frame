UPDATE legacy_mobile_upload_records_v1
SET lifecycle_state = 'provider_pending',
    updated_at_ms = ?4,
    last_operation_id = ?3
WHERE mapped_video_id = ?1
  AND actor_id = ?2
  AND lifecycle_state = 'uploading';
