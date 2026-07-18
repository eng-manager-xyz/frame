SELECT tenant_id, domain, phase, writer, epoch, phase_epoch, audit_head, updated_at_ms
FROM cutover_authority_scopes
WHERE tenant_id = ?1 AND domain = ?2
