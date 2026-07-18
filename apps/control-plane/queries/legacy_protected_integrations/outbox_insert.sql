INSERT INTO legacy_protected_integration_outbox_v1(
  receipt_id,provider_kind,payload_json,payload_digest,state,
  attempt_count,created_at_ms,completed_at_ms
) VALUES(
  ?1,?2,?3,?4,'pending_provider_evidence',0,?5,NULL
);
