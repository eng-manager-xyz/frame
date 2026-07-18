INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'authority', 1, COUNT(*)
FROM legacy_mobile_upload_records_v1 mobile
JOIN videos video ON video.id = mobile.mapped_video_id
JOIN video_uploads upload ON upload.id = mobile.upload_id
WHERE mobile.mapped_video_id = ?2
  AND mobile.legacy_video_id = ?3
  AND mobile.actor_id = ?4
  AND mobile.organization_id = ?5
  AND mobile.raw_file_key = ?6
  AND video.owner_id = ?4
  AND video.deleted_at_ms IS NULL
  AND upload.video_id = ?2;
