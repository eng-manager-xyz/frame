SELECT
  operation.operation_id,
  operation.organization_id,
  operation.actor_id,
  operation.action,
  operation.request_digest,
  operation.state,
  receipt.result_kind,
  receipt.invite_id,
  receipt.space_id,
  receipt.creator_id,
  receipt.actor_authority,
  receipt.matching_before,
  receipt.deleted_rows,
  receipt.inserted_rows,
  receipt.matching_after,
  receipt.result_count,
  effect.invalidates_organization_invites,
  effect.invalidates_space_page,
  effect.invalidates_space_members,
  effect.bumps_authority_generation,
  effect.authority_subject_count,
  effect.revalidation_path,
  (SELECT COUNT(*) FROM legacy_membership_action_audit_events_v1 audit
    WHERE audit.operation_id = operation.operation_id) AS audit_count,
  (SELECT COUNT(*) FROM legacy_membership_action_proof_consumptions_v1 proof
    WHERE proof.related_operation_id = operation.operation_id) AS proof_count
FROM legacy_membership_action_operations_v1 operation
LEFT JOIN legacy_membership_action_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
LEFT JOIN legacy_membership_action_effects_v1 effect
  ON effect.operation_id = operation.operation_id
WHERE operation.organization_id = ?1
  AND operation.actor_id = ?2
  AND operation.action = ?3
  AND operation.idempotency_key_digest = ?4
LIMIT 2
