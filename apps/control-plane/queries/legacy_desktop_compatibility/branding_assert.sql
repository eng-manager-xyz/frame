INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1, COUNT(*)
FROM organizations
WHERE id = ?2
  AND legacy_desktop_metadata_json = ?3
  AND legacy_desktop_icon_url IS ?4
  AND revision = ?5
  AND legacy_desktop_branding_revision = ?6
  AND legacy_desktop_branding_last_operation_id = ?1;
