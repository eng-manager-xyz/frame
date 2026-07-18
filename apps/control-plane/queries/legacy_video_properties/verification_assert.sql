INSERT INTO legacy_video_property_assertions_v1 (
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'verification', 1, COUNT(*)
FROM videos v
WHERE v.id = ?2
  AND v.deleted_at_ms IS NULL
  AND v.legacy_password_hash IS ?3
  AND v.legacy_property_revision = ?4
  AND (
    SELECT COUNT(*) FROM space_videos sv
    JOIN spaces s ON s.id = sv.space_id
    WHERE sv.video_id = v.id AND s.deleted_at_ms IS NULL
  ) = ?5
  AND COALESCE((
    SELECT group_concat(snapshot, '|') FROM (
      SELECT s.id || ':' || s.legacy_password_revision || ':' ||
             COALESCE(s.legacy_password_hash, '') AS snapshot
      FROM space_videos sv
      JOIN spaces s ON s.id = sv.space_id
      WHERE sv.video_id = v.id AND s.deleted_at_ms IS NULL
      ORDER BY sv.rowid
    )
  ), '') = ?6;
