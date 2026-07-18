UPDATE organizations
SET legacy_icon_key = ?3,
    legacy_desktop_icon_url = ?3,
    legacy_desktop_branding_revision = legacy_desktop_branding_revision + 1,
    legacy_desktop_branding_last_operation_id = ?4,
    updated_at_ms = MAX(updated_at_ms, ?5),
    revision = revision + 1,
    last_operation_id = ?4
WHERE id = ?1
  AND status = 'active'
  AND EXISTS (
    SELECT 1 FROM organization_members member
    WHERE member.organization_id = organizations.id
      AND member.user_id = ?2
      AND member.state = 'active'
      AND member.role IN ('owner', 'admin')
  );
