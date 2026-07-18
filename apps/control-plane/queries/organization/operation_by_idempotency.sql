SELECT operation.operation_id,
       operation.organization_id,
       operation.idempotency_key,
       operation.operation_kind,
       operation.subject_id,
       operation.request_fingerprint,
       operation.result_code,
       operation.resulting_revision,
       operation.authority_version,
       operation.committed_at_ms
FROM organization_repository_operations_v1 operation
JOIN organization_audit_events_v1 audit
  ON audit.operation_id = operation.operation_id
 AND audit.organization_id = operation.organization_id
 AND audit.actor_id = ?3
 AND audit.outcome = 'allow'
JOIN users actor ON actor.id = audit.actor_id AND actor.status = 'active'
JOIN auth_identities_v2 identity
  ON identity.user_id = actor.id
 AND identity.identity_revision = ?4
 AND identity.session_version = ?5
WHERE operation.organization_id = ?1 AND operation.idempotency_key = ?2
LIMIT 1
