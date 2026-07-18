INSERT INTO legacy_developer_action_receipts_v1(
  operation_id, result_kind, app_id, legacy_app_id, final_name,
  final_environment, final_logo_url, update_statement_executed,
  deleted_at_ms, revoked_active_key_count, active_key_count_after,
  domain_id, legacy_domain_id, stored_origin, matched_rows, video_id, account_present,
  auto_top_up_enabled, auto_top_up_threshold_microcredits,
  auto_top_up_amount_cents, credit_account_id, public_key_id,
  secret_key_id, sealed_key_replay, replay_binding, created_at_ms
)
VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
  ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
)
