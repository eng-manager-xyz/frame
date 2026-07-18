SELECT
  receipt.receipt_id,
  receipt.request_digest,
  receipt.state,
  receipt.provider_kind,
  receipt.human_approval_required,
  human.decision AS human_decision,
  provider.sealed_response_ref,
  provider.sealed_response_digest
FROM legacy_protected_billing_auth_generated_replay_claims_v1 claim
JOIN legacy_protected_billing_auth_receipts_v1 receipt
  ON receipt.receipt_id = claim.receipt_id
JOIN legacy_protected_billing_auth_live_authority_v1 live
  ON live.receipt_id = receipt.receipt_id
LEFT JOIN legacy_protected_billing_auth_human_evidence_v1 human
  ON human.receipt_id = receipt.receipt_id
LEFT JOIN legacy_protected_billing_auth_provider_evidence_v1 provider
  ON provider.receipt_id = receipt.receipt_id
WHERE claim.source_operation_id = ?1
  AND claim.principal_digest = ?2
  AND claim.request_digest = ?3
  AND receipt.replay_origin = 'generated'
  AND (
    receipt.state IN ('awaiting_human_approval','awaiting_provider_evidence')
    OR (
      receipt.state IN ('verified','rejected','dead_letter')
      AND receipt.completed_at_ms IS NOT NULL
      AND receipt.completed_at_ms > ?4
    )
  )
  AND live.authority_expires_at_ms > ?5
LIMIT 1;
