INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'actor_authority', 1,
  (
    SELECT COUNT(*) FROM users actor
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND actor.organization_preference_revision = ?3
  )
)
