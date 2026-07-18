SELECT
  receipt.actor_id,
  receipt.target_id,
  receipt.credential_kind,
  receipt.credential_subject_id,
  receipt.credential_key_version,
  receipt.credential_digest
FROM legacy_protected_billing_auth_receipts_v1 receipt
JOIN legacy_protected_billing_auth_live_authority_v1 live
  ON live.receipt_id = receipt.receipt_id
JOIN legacy_protected_billing_auth_outbox_v1 outbox
  ON outbox.receipt_id = receipt.receipt_id
JOIN legacy_protected_billing_auth_approval_requests_v1 approval
  ON approval.receipt_id = receipt.receipt_id
WHERE receipt.receipt_id = ?1
  AND receipt.request_digest = ?2
  AND receipt.source_operation_id = 'cap-v1-14ea978608dcf07e'
  AND receipt.operation_kind = 'server_action'
  AND receipt.method = 'ACTION'
  AND receipt.auth_class = 'admin_session'
  AND receipt.authority_class = 'messenger_admin_video'
  AND receipt.provider_kind = 'media_reprocess_workflow_dispatch'
  AND receipt.human_approval_required = 1
  AND receipt.actor_id IS NOT NULL
  AND receipt.target_id IS NOT NULL
  AND receipt.credential_kind = 'session_token'
  AND receipt.credential_subject_id IS NOT NULL
  AND receipt.credential_key_version IS NOT NULL
  AND receipt.credential_digest IS NOT NULL
  AND live.authority_expires_at_ms > ?3
  AND receipt.state NOT IN ('rejected','dead_letter')
  AND outbox.provider_kind = receipt.provider_kind
  AND approval.request_digest = receipt.request_digest
LIMIT 1;
