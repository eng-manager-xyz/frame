UPDATE authenticated_web_action_operations_v1
SET state = 'complete', response_json = ?2, completed_at_ms = ?3
WHERE operation_id = ?1
  AND state = 'claimed'
