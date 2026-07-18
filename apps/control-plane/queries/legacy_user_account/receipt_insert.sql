INSERT INTO legacy_user_account_receipts_v1(
  operation_id, actor_id, action, result_kind, onboarding_step,
  result_legacy_organization_id, provider_effect,
  resulting_user_revision, created_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9);
