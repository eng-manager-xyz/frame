INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'selection_authority',
  1,
  (
    SELECT COUNT(*)
    FROM users
    WHERE id = ?2
      AND status = 'active'
      AND deleted_at_ms IS NULL
      AND active_organization_id = ?3
      AND organization_preference_revision = ?4
  )
)
