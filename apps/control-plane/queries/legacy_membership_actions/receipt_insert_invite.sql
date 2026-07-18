INSERT INTO legacy_membership_action_receipts_v1(
  operation_id, result_kind, invite_id, actor_authority,
  matching_before, deleted_rows, inserted_rows, matching_after, created_at_ms
)
VALUES (?1, 'organization_invite_removed', ?2, ?3, 1, 1, 0, 0, ?4)
