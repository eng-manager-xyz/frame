SELECT
  operation_id,
  request_digest,
  state,
  result_kind,
  onboarding_step,
  result_legacy_organization_id,
  provider_effect,
  (SELECT COUNT(*) FROM legacy_user_account_receipts_v1 r
    WHERE r.operation_id = o.operation_id) AS receipt_count,
  (SELECT COUNT(*) FROM legacy_user_account_effects_v1 e
    WHERE e.operation_id = o.operation_id) AS effect_count,
  (SELECT COUNT(*) FROM legacy_user_account_audit_events_v1 a
    WHERE a.operation_id = o.operation_id) AS audit_count
FROM legacy_user_account_operations_v1 o
WHERE actor_id = ?1
  AND action = ?2
  AND idempotency_key_digest = ?3
LIMIT 1;
