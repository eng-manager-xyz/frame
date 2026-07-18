SELECT parent.source_operation_id,parent.actor_id,parent.tenant_id,parent.target_id,
       parent.credential_kind,parent.credential_subject_id,parent.credential_key_version,
       parent.credential_digest,parent.policy_proofs_json,
       parent.entitlement_kind,parent.entitlement_subject_id,parent.entitlement_revision,
       parent.entitlement_expires_at_ms,parent.authority_binding_digest,
       parent.created_at_ms,edge.target_binding_rule,
       CASE
         WHEN parent.parent_family='protected_integrations'
          AND parent.source_operation_id='cap-v1-d9b654b30f6c362a'
         THEN (
           SELECT alias.legacy_video_id
           FROM legacy_collaboration_video_aliases_v1 alias
           WHERE alias.mapped_video_id=parent.target_id
         )
         ELSE NULL
       END AS translated_legacy_target_id
FROM legacy_protected_effect_parent_registry_v1 parent
JOIN legacy_protected_effect_parent_edges_v1 edge
  ON edge.parent_family=parent.parent_family
 AND edge.parent_operation_id=parent.source_operation_id
 AND edge.child_family='protected_media'
 AND edge.child_operation_id=?4
WHERE parent.parent_family=?1 AND parent.parent_receipt_id=?2
  AND parent.request_digest=?3 AND parent.state<>'dead_letter'
  AND parent.created_at_ms<=?5
LIMIT 1;
