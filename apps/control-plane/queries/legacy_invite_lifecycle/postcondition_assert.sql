INSERT INTO legacy_invite_lifecycle_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'durable_exact_postcondition', 1,
  CASE WHEN
    NOT EXISTS (
      SELECT 1 FROM organization_invites
      WHERE id = ?4 AND organization_id = ?3
    )
    AND EXISTS (
      SELECT 1 FROM legacy_invite_lifecycle_invite_aliases_v1
      WHERE mapped_invite_id = ?4
        AND organization_id = ?3
        AND decision = ?5
        AND last_operation_id = ?1
    )
    AND EXISTS (
      SELECT 1 FROM legacy_invite_lifecycle_receipts_v1
      WHERE operation_id = ?1 AND action = ?6
    )
    AND EXISTS (
      SELECT 1 FROM legacy_invite_lifecycle_audit_events_v1
      WHERE operation_id = ?1 AND actor_id = ?2 AND organization_id = ?3
    )
    AND (
      (?5 = 'accepted' AND EXISTS (
        SELECT 1 FROM organization_members
        WHERE organization_id = ?3 AND user_id = ?2 AND state = 'active'
      ) AND EXISTS (
        SELECT 1 FROM users
        WHERE id = ?2
          AND active_organization_id = ?3
          AND json_extract(legacy_onboarding_steps_json, '$.organizationSetup') = 1
          AND json_extract(legacy_onboarding_steps_json, '$.customDomain') = 1
          AND json_extract(legacy_onboarding_steps_json, '$.inviteTeam') = 1
      ))
      OR
      (?5 = 'declined' AND NOT EXISTS (
        SELECT 1 FROM organization_members
        WHERE organization_id = ?3 AND user_id = ?2 AND state = 'active'
      ) AND NOT EXISTS (
        SELECT 1 FROM space_members member
        JOIN spaces space ON space.id = member.space_id
        WHERE member.user_id = ?2 AND space.organization_id = ?3
      ))
    )
  THEN 1 ELSE 0 END;
