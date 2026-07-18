INSERT INTO legacy_analytics_provider_operations_v1(
  operation_id, source_operation_id, operation_kind, principal_digest,
  actor_id, active_organization_id, target_video_id, execution_key_digest,
  request_digest, state, created_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10)
