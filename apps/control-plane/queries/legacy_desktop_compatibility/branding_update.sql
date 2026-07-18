UPDATE organizations
SET legacy_desktop_metadata_json = ?4,
    legacy_desktop_icon_url = CASE ?5 WHEN 0 THEN legacy_desktop_icon_url ELSE ?6 END,
    updated_at_ms = ?7,
    revision = revision + 1,
    legacy_desktop_branding_revision = legacy_desktop_branding_revision + 1,
    legacy_desktop_branding_last_operation_id = ?8
WHERE id = ?1
  AND revision = ?2
  AND legacy_desktop_branding_revision = ?3
  AND status = 'active'
  AND tombstoned_at_ms IS NULL
  AND (
    owner_id = ?9
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = organizations.id
        AND member.user_id = ?9
        AND member.state = 'active'
        AND member.role = 'admin'
    )
  );
