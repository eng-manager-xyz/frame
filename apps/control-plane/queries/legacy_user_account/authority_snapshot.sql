SELECT
  id,
  display_name,
  legacy_last_name,
  legacy_onboarding_steps_json,
  active_organization_id,
  default_organization_id,
  session_version,
  legacy_user_account_revision,
  legacy_user_account_authority_version
FROM users
WHERE id = ?1
  AND status = 'active'
  AND deleted_at_ms IS NULL
LIMIT 1;
