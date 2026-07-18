INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'creator_graph', 1,
  (
    SELECT COUNT(*)
    FROM users creator
    JOIN organizations organization
      ON organization.id = ?2
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = creator.id
    WHERE creator.id = ?3
      AND creator.status = 'active'
      AND creator.deleted_at_ms IS NULL
      AND (
        organization.owner_id = creator.id
        OR membership.state = 'active'
      )
  )
)
