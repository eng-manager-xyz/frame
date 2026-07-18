UPDATE outbox_events
SET state = ?4,
    event_sequence = ?3,
    event_fingerprint = ?5,
    revision = revision + 1,
    attempt = CASE WHEN ?4 = 'leased' THEN attempt + 1 ELSE attempt END,
    lease_expires_at_ms = ?6,
    delivered_at_ms = CASE WHEN ?4 = 'delivered' THEN ?7 ELSE delivered_at_ms END,
    last_operation_id = ?8
WHERE id = ?1 AND organization_id = ?2 AND event_sequence + 1 = ?3
