SELECT password_hash, ordinal FROM (
  SELECT v.legacy_password_hash AS password_hash, 0 AS ordinal
  FROM videos v WHERE v.id = ?1 AND v.deleted_at_ms IS NULL
  UNION ALL
  SELECT s.legacy_password_hash AS password_hash,
         ROW_NUMBER() OVER (ORDER BY sv.rowid) AS ordinal
  FROM space_videos sv
  JOIN spaces s ON s.id = sv.space_id
  WHERE sv.video_id = ?1 AND s.deleted_at_ms IS NULL
)
WHERE password_hash IS NOT NULL
ORDER BY ordinal;
