UPDATE legacy_user_account_operations_v1
SET state = 'applied',
    result_kind = ?2,
    onboarding_step = ?3,
    result_legacy_organization_id = ?4,
    provider_effect = ?5,
    completed_at_ms = ?6
WHERE operation_id = ?1 AND state = 'pending';
