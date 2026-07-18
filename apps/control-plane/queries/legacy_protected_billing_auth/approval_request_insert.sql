INSERT INTO legacy_protected_billing_auth_approval_requests_v1(
  receipt_id,approval_scope,request_digest,state,created_at_ms,resolved_at_ms
) VALUES(?1,'billing_admin.v1',?2,'pending',?3,NULL);
