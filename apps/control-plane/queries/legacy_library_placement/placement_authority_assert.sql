INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'product_effect',
  1,
  CASE WHEN
    (?5 = '' AND ?4 IN ('owner', 'admin'))
    OR (?5 <> '' AND EXISTS (
        SELECT 1
        FROM spaces space
        LEFT JOIN space_members membership
          ON membership.space_id = space.id
         AND membership.user_id = ?3
         AND membership.state = 'active'
        WHERE space.id = ?5
          AND space.organization_id = ?2
          AND space.deleted_at_ms IS NULL
          AND space.revision = ?6
          AND space.authority_version = ?7
          AND COALESCE(membership.role, '') = ?9
          AND COALESCE(membership.revision, -1) = ?8
          AND (
            ?4 IN ('owner', 'admin')
            OR (?4 = 'member' AND COALESCE(membership.role, '') = 'manager')
          )
      ))
  THEN 1 ELSE 0 END
)
