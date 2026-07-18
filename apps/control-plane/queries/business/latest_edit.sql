SELECT id, video_id, document_version, edit_spec_json, document_checksum,
       created_by_user_id, created_at_ms, updated_at_ms, revision
FROM video_edits
WHERE video_id = ?1
ORDER BY document_version DESC, id DESC
LIMIT 1
