INSERT OR IGNORE INTO legacy_transcript_storage_receipts_v1(
  operation_id, object_key, prior_etag, applied_etag,
  content_sha256, content_bytes, applied_at_ms
) VALUES(?1,?2,?3,?4,?5,?6,?7);
