INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'provider_pending', 1, COUNT(*)
FROM legacy_mobile_upload_records_v1 mobile
JOIN legacy_mobile_upload_processing_intents_v1 intent
  ON intent.mapped_video_id = mobile.mapped_video_id
JOIN legacy_mobile_upload_operations_v1 operation
  ON operation.operation_id = intent.operation_id
WHERE mobile.mapped_video_id = ?2
  AND mobile.lifecycle_state = 'provider_pending'
  AND intent.operation_id = ?1
  AND intent.state = 'provider_pending'
  AND intent.observed_bytes = ?3
  AND operation.state = 'provider_pending';
