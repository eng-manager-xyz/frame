INSERT INTO legacy_notification_action_receipts_v1(
  operation_id, result_kind, selected_notification_id, matched_count, read_at_ms,
  notifications_json, preserved_before_sha256, preserved_after_sha256,
  matching_before, updated_rows, matching_after, out_of_scope_updated_rows,
  other_actor_rows_updated, created_at_ms
)
VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
)
