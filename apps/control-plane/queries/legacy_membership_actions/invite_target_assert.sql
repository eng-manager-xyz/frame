INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'invite_target', 1,
  (SELECT COUNT(*) FROM organization_invites invite
    WHERE invite.id = ?2 AND invite.organization_id = ?3)
)
