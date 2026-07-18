INSERT INTO legacy_protected_billing_auth_outbox_v1(
  receipt_id,provider_kind,payload_json,payload_digest,state,
  attempt_count,created_at_ms,completed_at_ms
) VALUES(
  ?1,?2,?3,?4,
  CASE WHEN ?5 = 1 THEN 'blocked_human_approval'
       ELSE 'pending_provider_evidence' END,
  0,?6,NULL
);
