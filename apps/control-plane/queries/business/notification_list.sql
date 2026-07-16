SELECT id, recipient_user_id, type, deduplication_key, data_json,
       payload_schema_version, payload_checksum, created_at_ms, read_at_ms
FROM notifications
WHERE organization_id=?1 AND recipient_user_id=?2
ORDER BY created_at_ms DESC, id DESC
LIMIT 1000
