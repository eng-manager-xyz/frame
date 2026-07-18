UPDATE business_messenger_legacy_quarantine_v1
SET disposition='purged', purged_at_ms=?3, last_operation_id=?4
WHERE organization_id=?2
  AND source_table || ':' || source_id=?1
  AND disposition='quarantined'
  AND purge_after_ms<=?3
