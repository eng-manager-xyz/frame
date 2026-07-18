INSERT INTO legacy_membership_action_receipts_v1(
  operation_id, result_kind, space_id, creator_id, actor_authority,
  matching_before, deleted_rows, inserted_rows, matching_after,
  result_count, created_at_ms
)
SELECT
  ?1, 'space_members_set', ?2, ?3, ?4,
  (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1),
  (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1),
  (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 WHERE operation_id = ?1),
  (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 WHERE operation_id = ?1),
  (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 WHERE operation_id = ?1),
  ?5
