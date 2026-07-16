//! Compile-time inventory for the token-gated business repository harness.
//!
//! Provider execution is exposed only when a deployment explicitly wires a
//! private conformance route. The offline SQLite suite is the default evidence.

pub const BUSINESS_CONFORMANCE_SCENARIOS: &[&str] = &[
    "read_authority_first_statement",
    "cross_tenant_denial_no_mutation",
    "semantic_replay_and_mismatch",
    "tenant_scoped_outbox_and_usage_idempotency",
    "comment_anonymous_privacy",
    "comment_list_and_tombstone",
    "share_owner_and_admin_policy",
    "share_exact_immutable_postcondition",
    "notification_recipient_list_and_mark_read",
    "notification_duplicate_and_gap",
    "same_key_deferred_event_convergence",
    "import_duplicate_and_gap",
    "upload_initial_and_ordered_lifecycle",
    "storage_manifest_reconciliation",
    "storage_exact_immutable_postcondition",
    "derivative_manifest_parity",
    "derivative_exact_immutable_postcondition",
    "developer_key_digest_only",
    "credit_account_read",
    "credit_append_and_balance",
    "usage_reconciliation",
    "owner_only_tenant_export_rows",
    "data_subject_tenant_binding",
    "legal_hold_place_and_release",
    "class_specific_delete_execution",
    "immutable_ledger_compensation",
    "messenger_excluded_fail_closed",
    "pinned_cap_schema_and_id_mapping",
    "dirty_upgrade_audit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_stable_and_unique() {
        let mut names = BUSINESS_CONFORMANCE_SCENARIOS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUSINESS_CONFORMANCE_SCENARIOS.len());
        assert!(names.iter().all(|name| !name.is_empty()));
    }
}
