INSERT INTO organization_repair_plans_v1(
  id, organization_id, generated_by_user_id, support_authority_fingerprint,
  findings_json, steps_json, dry_run, generated_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
