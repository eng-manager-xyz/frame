SELECT parent.source_operation_id,parent.actor_id,parent.tenant_id,parent.target_id,
       parent.auth_class,parent.authority_class,
       parent.credential_kind,parent.credential_subject_id,parent.credential_key_version,
       parent.credential_digest,parent.credential_expires_at_ms,parent.policy_proofs_json,
       parent.entitlement_kind,parent.entitlement_subject_id,parent.entitlement_revision,
       parent.entitlement_expires_at_ms,parent.authority_binding_digest,
       edge.target_binding_rule
FROM legacy_protected_effect_parent_registry_v1 parent
JOIN legacy_protected_effect_parent_edges_v1 edge
  ON edge.parent_family=parent.parent_family
 AND edge.parent_operation_id=parent.source_operation_id
 AND edge.child_family='protected_integrations'
 AND edge.child_operation_id=?4
WHERE parent.parent_family=?1 AND parent.parent_receipt_id=?2
  AND parent.request_digest=?3 AND parent.state<>'dead_letter'
LIMIT 1;
