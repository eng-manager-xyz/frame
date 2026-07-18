INSERT INTO video_edits(
  id, video_id, document_version, edit_spec_json, created_by_user_id,
  created_at_ms, updated_at_ms, revision, document_checksum, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?11)
ON CONFLICT(id) DO UPDATE SET
  document_version = excluded.document_version,
  edit_spec_json = excluded.edit_spec_json,
  updated_at_ms = excluded.updated_at_ms,
  revision = excluded.revision,
  document_checksum = excluded.document_checksum,
  last_operation_id = excluded.last_operation_id
WHERE video_edits.video_id = excluded.video_id
  AND video_edits.created_by_user_id = excluded.created_by_user_id
  AND video_edits.created_at_ms = excluded.created_at_ms
  AND video_edits.revision = ?10
