SELECT receipt.receipt_id,receipt.request_digest,receipt.state,receipt.provider_kind,
       evidence.sealed_terminal_ref,evidence.sealed_terminal_digest
FROM legacy_protected_integration_generated_replay_claims_v1 claim
JOIN legacy_protected_integration_receipts_v1 receipt ON receipt.receipt_id=claim.receipt_id
JOIN legacy_protected_integration_live_authority_v1 live ON live.receipt_id=receipt.receipt_id
LEFT JOIN legacy_protected_integration_evidence_v1 evidence ON evidence.receipt_id=receipt.receipt_id
WHERE claim.source_operation_id=?1
  AND claim.principal_digest=?2
  AND claim.request_digest=?3
  AND receipt.authority_binding_digest=?4
  AND live.authority_expires_at_ms>?5
  AND receipt.replay_origin='generated'
  AND (receipt.operation_kind<>'workflow' OR EXISTS (
    SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
    WHERE parent.parent_family=receipt.parent_family
      AND parent.parent_receipt_id=receipt.parent_receipt_id
      AND parent.request_digest=receipt.parent_request_digest
      AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
      AND parent.state<>'dead_letter'
  ))
  AND (
    receipt.state='pending_provider_evidence'
    OR (receipt.state IN ('verified','dead_letter')
      AND receipt.completed_at_ms IS NOT NULL AND receipt.completed_at_ms>?6)
  )
  AND (receipt.state<>'verified' OR evidence.terminal_expires_at_ms>?5)
LIMIT 1;
