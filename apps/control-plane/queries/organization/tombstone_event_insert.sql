INSERT INTO organization_tombstone_events_v1(
  operation_id, organization_id, actor_id, event_kind, occurred_at_ms, retention_until_ms
)
SELECT ?1, o.id, ?3, ?4,
       CASE ?4
         WHEN 'tombstoned' THEN o.tombstoned_at_ms
         WHEN 'recovered' THEN o.recovered_at_ms
       END,
       CASE ?4 WHEN 'tombstoned' THEN o.retention_until_ms ELSE NULL END
FROM organizations o
WHERE o.id = ?2
  AND o.last_operation_id = ?1
  AND ((?4 = 'tombstoned' AND o.tombstoned_at_ms IS NOT NULL)
       OR (?4 = 'recovered' AND o.recovered_at_ms IS NOT NULL))
