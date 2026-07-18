INSERT INTO cutover_authority_audit(
  audit_hash, previous_hash, tenant_id, domain, action,
  from_phase, to_phase, from_epoch, to_epoch,
  operator_digest, evidence_digest, occurred_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
