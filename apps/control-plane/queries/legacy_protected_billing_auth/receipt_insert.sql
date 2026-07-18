INSERT INTO legacy_protected_billing_auth_receipts_v1(
  receipt_id,source_operation_id,operation_kind,method,surface_path,
  auth_class,authority_class,provider_kind,human_approval_required,
  provider_execution_required,principal_digest,actor_id,
  credential_kind,credential_subject_id,credential_key_version,credential_digest,
  sealed_request_ref,sealed_request_digest,target_id,
  replay_key_digest,replay_origin,idempotency_mode,request_digest,redacted_request_json,
  state,created_at_ms,completed_at_ms
) VALUES(
  ?1,?2,?3,?4,?5,
  ?6,?7,?8,?9,
  1,?10,?11,?12,?13,?14,?15,?16,?17,?18,
  ?19,?20,?21,?22,?23,
  CASE WHEN ?9 = 1 THEN 'awaiting_human_approval'
       ELSE 'awaiting_provider_evidence' END,
  ?24,NULL
);
