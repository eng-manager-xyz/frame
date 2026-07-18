INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'postcondition', 1, COUNT(*)
FROM legacy_mobile_upload_records_v1 mobile
JOIN videos video ON video.id = mobile.mapped_video_id
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN video_uploads upload ON upload.id = mobile.upload_id
JOIN legacy_mobile_cap_media_v1 media ON media.mapped_video_id = video.id
JOIN legacy_mobile_cap_uploads_v1 progress ON progress.mapped_video_id = video.id
JOIN legacy_mobile_upload_operations_v1 operation
  ON operation.operation_id = ?1 AND operation.mapped_video_id = video.id
WHERE mobile.mapped_video_id = ?2
  AND mobile.legacy_video_id = ?3
  AND mobile.actor_id = ?4
  AND mobile.organization_id = ?5
  AND mobile.raw_file_key = ?6
  AND mobile.upload_id = ?7
  AND video.owner_id = ?4
  AND video_alias.legacy_video_id = ?3
  AND upload.video_id = ?2
  AND upload.organization_id = ?5
  AND media.legacy_video_id = ?3
  AND progress.phase = 'uploading';
