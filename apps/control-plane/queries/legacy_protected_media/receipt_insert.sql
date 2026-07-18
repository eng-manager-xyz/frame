INSERT INTO legacy_protected_media_receipts_v1 (
  receipt_id,source_operation_id,operation_kind,method,surface_path,
  auth_class,authority_class,principal_digest,actor_id,tenant_id,
  credential_kind,credential_subject_id,credential_key_version,credential_digest,
  policy_proofs_json,entitlement_kind,entitlement_subject_id,
  entitlement_revision,entitlement_expires_at_ms,target_id,authority_binding_digest,
  parent_family,parent_receipt_id,parent_request_digest,
  parent_authority_binding_digest,execution_key_digest,
  replay_origin,idempotency_mode,request_digest,payload_digest,
  request_descriptor_json,sealed_request_ref,sealed_request_digest,
  terminal_kind,executor_kind,provider_required,state,created_at_ms
) VALUES (
  ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
  ?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,
  ?36,'pending_execution_evidence',?37
);
