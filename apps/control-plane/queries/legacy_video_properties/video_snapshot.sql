SELECT
  v.id,
  v.owner_id,
  v.title,
  v.created_at_ms,
  v.updated_at_ms,
  v.duration_ms,
  f.legacy_folder_id AS folder_id,
  v.legacy_public,
  v.legacy_password_hash AS password_hash,
  v.legacy_metadata_json AS metadata_json,
  v.legacy_settings_json AS settings_json,
  v.revision,
  v.legacy_property_revision AS property_revision,
  COALESCE(u.display_name, '') AS owner_name,
  (
    SELECT COUNT(*) FROM comments c
    WHERE c.video_id = v.id AND c.deleted_at_ms IS NULL
  ) + (
    SELECT COUNT(*) FROM legacy_collaboration_comments_v1 lc
    WHERE lc.mapped_video_id = v.id AND lc.comment_kind = 'text'
  ) AS comment_count,
  (
    SELECT COUNT(*) FROM legacy_collaboration_comments_v1 lr
    WHERE lr.mapped_video_id = v.id AND lr.comment_kind = 'emoji'
  ) AS reaction_count,
  (
    SELECT vu.received_bytes FROM video_uploads vu
    WHERE vu.video_id = v.id ORDER BY vu.created_at_ms DESC, vu.id DESC LIMIT 1
  ) AS uploaded_bytes,
  (
    SELECT vu.expected_bytes FROM video_uploads vu
    WHERE vu.video_id = v.id ORDER BY vu.created_at_ms DESC, vu.id DESC LIMIT 1
  ) AS total_bytes,
  (
    SELECT vu.state FROM video_uploads vu
    WHERE vu.video_id = v.id ORDER BY vu.created_at_ms DESC, vu.id DESC LIMIT 1
  ) AS upload_phase,
  (
    SELECT COUNT(*) FROM space_videos sv
    JOIN spaces s ON s.id = sv.space_id
    WHERE sv.video_id = v.id AND s.deleted_at_ms IS NULL
  ) AS joined_space_count,
  COALESCE((
    SELECT group_concat(snapshot, '|') FROM (
      SELECT s.id || ':' || s.legacy_password_revision || ':' ||
             COALESCE(s.legacy_password_hash, '') AS snapshot
      FROM space_videos sv
      JOIN spaces s ON s.id = sv.space_id
      WHERE sv.video_id = v.id AND s.deleted_at_ms IS NULL
      ORDER BY sv.rowid
    )
  ), '') AS joined_space_snapshot
FROM videos v
JOIN users u ON u.id = v.owner_id
LEFT JOIN folders f ON f.id = v.folder_id
WHERE v.deleted_at_ms IS NULL
  AND (
    v.id = ?2
    OR EXISTS (
      SELECT 1 FROM legacy_collaboration_video_aliases_v1 a
      WHERE a.mapped_video_id = v.id AND a.legacy_video_id = ?1
    )
  )
ORDER BY CASE WHEN v.id = ?2 THEN 0 ELSE 1 END
LIMIT 1;
