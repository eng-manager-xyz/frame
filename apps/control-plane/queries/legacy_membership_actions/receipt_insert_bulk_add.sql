INSERT INTO legacy_membership_action_receipts_v1(
  operation_id, result_kind, space_id, creator_id, actor_authority,
  matching_before, deleted_rows, inserted_rows, matching_after,
  result_count, created_at_ms
)
SELECT ?1, 'space_members_added', ?2, ?3, ?4,
  (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1),
  0,
  (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 final
    WHERE final.operation_id = ?1
      AND NOT EXISTS (
        SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
        WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
      )),
  (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1)
    + (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 final
        WHERE final.operation_id = ?1
          AND NOT EXISTS (
            SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
            WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
          )),
  (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 final
    WHERE final.operation_id = ?1
      AND NOT EXISTS (
        SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
        WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
      )),
  ?5
