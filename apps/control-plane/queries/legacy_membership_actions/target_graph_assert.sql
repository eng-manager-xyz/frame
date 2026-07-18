INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'target_graph', 0,
  (
    SELECT COUNT(*)
    FROM legacy_membership_action_final_members_v1 target
    WHERE target.operation_id = ?1
      AND NOT EXISTS (
        SELECT 1
        FROM users target_user
        JOIN organizations organization
          ON organization.id = ?2
         AND organization.status = 'active'
         AND organization.tombstoned_at_ms IS NULL
        LEFT JOIN organization_members membership
          ON membership.organization_id = organization.id
         AND membership.user_id = target_user.id
        WHERE target_user.id = target.user_id
          AND target_user.status = 'active'
          AND target_user.deleted_at_ms IS NULL
          AND (
            organization.owner_id = target_user.id
            OR membership.state = 'active'
          )
      )
  )
)
