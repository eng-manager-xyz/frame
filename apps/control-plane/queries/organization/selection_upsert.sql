UPDATE users
SET default_organization_id = ?3,
    active_organization_id = ?4,
    organization_preference_revision = organization_preference_revision + 1,
    organization_last_operation_id = ?5,
    updated_at_ms = ?6
WHERE id = ?1
  AND organization_preference_revision = ?2
  AND (?3 IS NULL OR EXISTS (
    SELECT 1 FROM organization_members m JOIN organizations o ON o.id = m.organization_id
    WHERE m.organization_id = ?3 AND m.user_id = ?1 AND m.state = 'active' AND o.status = 'active'
  ))
  AND (?4 IS NULL OR EXISTS (
    SELECT 1 FROM organization_members m JOIN organizations o ON o.id = m.organization_id
    WHERE m.organization_id = ?4 AND m.user_id = ?1 AND m.state = 'active' AND o.status = 'active'
  ))
