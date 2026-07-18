SELECT
  operation.operation_id,
  operation.request_digest,
  operation.state,
  receipt.result_kind,
  receipt.app_id,
  receipt.legacy_app_id,
  receipt.final_name,
  receipt.final_environment,
  receipt.final_logo_url,
  receipt.update_statement_executed,
  receipt.deleted_at_ms,
  receipt.revoked_active_key_count,
  receipt.active_key_count_after,
  receipt.domain_id,
  receipt.legacy_domain_id,
  receipt.stored_origin,
  receipt.matched_rows,
  receipt.video_id,
  receipt.account_present,
  receipt.auto_top_up_enabled,
  receipt.auto_top_up_threshold_microcredits,
  receipt.auto_top_up_amount_cents,
  receipt.credit_account_id,
  receipt.public_key_id,
  receipt.secret_key_id,
  receipt.sealed_key_replay,
  receipt.replay_binding,
  public_key.legacy_key_id AS public_legacy_key_id,
  public_key.key_prefix AS public_key_prefix,
  public_key.key_digest AS public_key_digest,
  public_key.encrypted_key AS public_encrypted_key,
  secret_key.legacy_key_id AS secret_legacy_key_id,
  secret_key.key_prefix AS secret_key_prefix,
  secret_key.key_digest AS secret_key_digest,
  secret_key.encrypted_key AS secret_encrypted_key,
  credit.legacy_credit_account_id,
  effect.revalidate_developer_dashboard,
  effect.revalidation_path,
  (SELECT COUNT(*) FROM legacy_developer_action_audit_events_v1 audit
    WHERE audit.operation_id = operation.operation_id
      AND audit.actor_id = operation.actor_id
      AND audit.action = operation.action
      AND audit.outcome = 'allow') AS audit_count,
  (SELECT COUNT(*) FROM legacy_developer_action_proof_consumptions_v1 proof
    WHERE proof.related_operation_id = operation.operation_id
      AND proof.actor_id = operation.actor_id
      AND proof.action = operation.action
      AND proof.outcome IN ('applied', 'replay')) AS proof_count
FROM legacy_developer_action_operations_v1 operation
LEFT JOIN legacy_developer_action_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
LEFT JOIN legacy_developer_action_effects_v1 effect
  ON effect.operation_id = operation.operation_id
LEFT JOIN legacy_developer_api_keys_v1 public_key
  ON public_key.id = receipt.public_key_id AND public_key.key_kind = 'public'
LEFT JOIN legacy_developer_api_keys_v1 secret_key
  ON secret_key.id = receipt.secret_key_id AND secret_key.key_kind = 'secret'
LEFT JOIN legacy_developer_credit_accounts_v1 credit
  ON credit.id = receipt.credit_account_id
WHERE operation.actor_id = ?1
  AND operation.action = ?2
  AND operation.idempotency_key_digest = ?3
LIMIT 2
