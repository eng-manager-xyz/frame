INSERT OR IGNORE INTO cutover_shadow_observations(
  observation_digest, tenant_id, domain, phase_epoch, query_class,
  normalization_digest, legacy_result_digest, d1_result_digest,
  classification, observed_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
