SELECT
  mobile.mapped_video_id,
  mobile.legacy_video_id,
  mobile.actor_id,
  mobile.organization_id,
  mobile.upload_id,
  mobile.raw_file_key,
  mobile.content_type,
  mobile.lifecycle_state,
  intent.operation_id AS intent_operation_id,
  intent.observed_bytes AS intent_observed_bytes,
  intent.requested_content_length AS intent_requested_content_length,
  intent.state AS intent_state
FROM legacy_mobile_upload_records_v1 mobile
JOIN videos video ON video.id = mobile.mapped_video_id
LEFT JOIN legacy_mobile_upload_processing_intents_v1 intent
  ON intent.mapped_video_id = mobile.mapped_video_id
WHERE mobile.actor_id = ?1
  AND mobile.legacy_video_id = ?2
  AND video.owner_id = ?1
  AND video.deleted_at_ms IS NULL
LIMIT 2;
