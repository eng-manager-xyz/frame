INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'membership_authority',
  1,
  (
    SELECT COUNT(*)
    FROM organization_members
    WHERE organization_id = ?2
      AND user_id = ?3
      AND role = ?4
      AND state = 'active'
      AND revision = ?5
      AND authority_version = ?6
  )
)
