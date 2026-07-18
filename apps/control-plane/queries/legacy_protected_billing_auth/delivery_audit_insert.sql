INSERT OR IGNORE INTO legacy_protected_billing_auth_delivery_audit_v1(
  receipt_id,transport_credential_digest,transport_body_digest,
  request_digest,observed_at_ms
) VALUES(?1,?2,?3,?4,?5);
