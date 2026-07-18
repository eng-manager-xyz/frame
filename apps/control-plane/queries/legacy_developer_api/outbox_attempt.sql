UPDATE legacy_developer_provider_outbox_v1
SET attempt_count = attempt_count + 1
WHERE operation_id = ?1 AND state = 'pending'
