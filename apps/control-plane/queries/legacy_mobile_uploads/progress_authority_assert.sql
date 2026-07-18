INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'authority', 1, COUNT(*)
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN video_uploads upload ON upload.id = ?5 AND upload.video_id = video.id
WHERE video.id = ?2
  AND video.owner_id = ?3
  AND video_alias.legacy_video_id = ?4
  AND video.deleted_at_ms IS NULL;
