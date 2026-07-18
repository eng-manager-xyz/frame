INSERT INTO business_event_inbox_v1(
  organization_id, aggregate_kind, aggregate_id, event_sequence,
  event_fingerprint, target_state, disposition, received_at_ms, operation_id
)
SELECT ?1,?2,?3,?4,?5,?6,
       CASE
         WHEN ?4 <= ?7 THEN 'stale'
         WHEN ?4 = ?7 + 1 THEN 'applied'
         ELSE 'deferred'
       END,
       ?8,?9
ON CONFLICT(organization_id, aggregate_kind, aggregate_id, event_sequence)
DO UPDATE SET
  disposition = CASE
    WHEN business_event_inbox_v1.disposition = 'deferred'
      AND excluded.disposition = 'applied'
      AND business_event_inbox_v1.event_fingerprint = excluded.event_fingerprint
    THEN 'applied'
    ELSE business_event_inbox_v1.disposition
  END,
  operation_id = CASE
    WHEN business_event_inbox_v1.disposition = 'deferred'
      AND excluded.disposition = 'applied'
      AND business_event_inbox_v1.event_fingerprint = excluded.event_fingerprint
    THEN excluded.operation_id
    ELSE business_event_inbox_v1.operation_id
  END
