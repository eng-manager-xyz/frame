UPDATE video_uploads
SET state=?4,
    received_bytes=?5,
    checksum_sha256=?6,
    event_sequence=?3,
    event_fingerprint=?7,
    updated_at_ms=?8,
    revision=revision+1,
    last_operation_id=?9
WHERE id=?1 AND organization_id=?2 AND event_sequence+1=?3
