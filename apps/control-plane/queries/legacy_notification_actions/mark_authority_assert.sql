INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'mark_authority',
  1,
  (
    SELECT COUNT(*)
    FROM users u
    JOIN organizations o
      ON o.id = u.active_organization_id
     AND o.id = ?3
     AND o.status = 'active'
     AND o.tombstoned_at_ms IS NULL
     AND o.revision = ?5
     AND o.authority_version = ?6
    JOIN organization_members m
      ON m.organization_id = o.id
     AND m.user_id = u.id
     AND m.state = 'active'
     AND m.role = ?7
     AND m.revision = ?8
     AND m.authority_version = ?9
    WHERE u.id = ?2
      AND u.status = 'active'
      AND u.deleted_at_ms IS NULL
      AND u.organization_preference_revision = ?4
      AND (
        (o.owner_id = u.id AND m.role = 'owner')
        OR (o.owner_id <> u.id AND m.role IN ('admin', 'member', 'viewer'))
      )
  )
)
