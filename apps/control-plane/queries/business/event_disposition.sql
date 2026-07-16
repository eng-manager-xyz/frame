SELECT disposition
FROM business_event_inbox_v1
WHERE organization_id = ?1 AND aggregate_kind = ?2
  AND aggregate_id = ?3 AND event_sequence = ?4
  AND event_fingerprint = ?5
LIMIT 2
