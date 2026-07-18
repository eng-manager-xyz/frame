UPDATE organizations
SET legacy_user_account_name = COALESCE(?2, legacy_user_account_name, name),
    name = CASE
      WHEN ?2 IS NULL THEN name
      WHEN length(?2) BETWEEN 1 AND 160 THEN ?2
      ELSE substr(?2, 1, 160)
    END,
    legacy_allowed_email_restriction = CASE WHEN ?3 = 0 THEN legacy_allowed_email_restriction ELSE ?4 END,
    updated_at_ms = ?6,
    revision = revision + 1,
    legacy_organization_library_revision = legacy_organization_library_revision + 1,
    legacy_organization_library_last_operation_id = ?5,
    last_operation_id = ?5
WHERE id = ?1 AND status = 'active' AND tombstoned_at_ms IS NULL
