INSERT INTO users(
  id, email, display_name, email_verified_at_ms, status,
  created_at_ms, updated_at_ms, active_organization_id,
  default_organization_id
) VALUES(?1, ?2, NULL, ?3, 'active', ?3, ?3, NULL, NULL)
