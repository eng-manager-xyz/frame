UPDATE legacy_developer_provider_outbox_v1
SET state = 'complete', completed_at_ms = ?2
WHERE operation_id = ?1 AND state = 'pending'
