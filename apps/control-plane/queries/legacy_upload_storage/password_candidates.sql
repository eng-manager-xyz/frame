SELECT password_hash, ordinal
FROM (
  SELECT video.legacy_password_hash AS password_hash, 0 AS ordinal
  FROM videos video
  WHERE video.id = ?1 AND video.legacy_password_hash IS NOT NULL

  UNION ALL

  SELECT space.legacy_password_hash AS password_hash,
         ROW_NUMBER() OVER (ORDER BY placement.space_id) - 1
           + CASE WHEN EXISTS (
               SELECT 1 FROM videos candidate_video
               WHERE candidate_video.id = ?1
                 AND candidate_video.legacy_password_hash IS NOT NULL
             ) THEN 1 ELSE 0 END AS ordinal
  FROM legacy_upload_storage_space_shares_v1 placement
  JOIN spaces space ON space.id = placement.space_id
  WHERE placement.mapped_video_id = ?1
    AND space.deleted_at_ms IS NULL
    AND space.legacy_password_hash IS NOT NULL
)
ORDER BY ordinal
LIMIT 1002;
