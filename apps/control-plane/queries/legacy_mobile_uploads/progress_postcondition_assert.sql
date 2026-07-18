INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1, COUNT(*)
FROM video_uploads upload
JOIN legacy_mobile_cap_uploads_v1 progress
  ON progress.mapped_video_id = upload.video_id
WHERE upload.id = ?2
  AND upload.video_id = ?3
  AND upload.received_bytes = ?4
  AND upload.expected_bytes = ?5
  AND progress.uploaded = CAST(?4 AS REAL)
  AND progress.total = CAST(?5 AS REAL);
