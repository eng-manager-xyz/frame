INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'organization_revision',
  1,
  (
    SELECT COUNT(*)
    FROM organizations
    WHERE id = ?2
      AND status = 'active'
      AND revision = ?3
      AND authority_version = ?4
  )
)
