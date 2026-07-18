SELECT COUNT(*) AS copied
FROM legacy_video_lifecycle_copy_receipts_v1
WHERE operation_id = ?1 AND source_key = ?2;
