SELECT receipt.receipt_id, receipt.request_digest, receipt.state,
  receipt.provider_required, receipt.terminal_kind,
  evidence.sealed_terminal_ref, evidence.sealed_terminal_digest,
  evidence.terminal_expires_at_ms
FROM legacy_protected_media_generated_replay_claims_v1 claim
JOIN legacy_protected_media_receipts_v1 receipt ON receipt.receipt_id=claim.receipt_id
JOIN legacy_protected_media_live_authority_v1 live ON live.receipt_id=receipt.receipt_id
LEFT JOIN legacy_protected_media_execution_evidence_v1 evidence
  ON evidence.receipt_id=receipt.receipt_id
WHERE claim.source_operation_id=?1
  AND claim.principal_digest=?2
  AND claim.request_digest=?3
  AND receipt.authority_binding_digest=?4
  AND live.authority_expires_at_ms>?5
  AND (receipt.state='pending_execution_evidence'
    OR receipt.completed_at_ms>?5-900000)
  AND (receipt.operation_kind<>'workflow' OR EXISTS (
    SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
    JOIN legacy_protected_effect_parent_edges_v1 edge
      ON edge.parent_family=parent.parent_family
     AND edge.parent_operation_id=parent.source_operation_id
     AND edge.child_family='protected_media'
     AND edge.child_operation_id=receipt.source_operation_id
    WHERE parent.parent_family=receipt.parent_family
      AND parent.parent_receipt_id=receipt.parent_receipt_id
      AND parent.request_digest=receipt.parent_request_digest
      AND parent.state<>'dead_letter'
      AND parent.actor_id IS receipt.actor_id
      AND parent.tenant_id IS receipt.tenant_id
      AND (
        (edge.target_binding_rule='same' AND parent.target_id IS receipt.target_id)
        OR (edge.target_binding_rule='child_derived' AND receipt.target_id IS NOT NULL)
      )
      AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
  ))
LIMIT 1;
