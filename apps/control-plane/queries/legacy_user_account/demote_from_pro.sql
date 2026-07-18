UPDATE users
SET legacy_stripe_customer_id = NULL,
    legacy_stripe_subscription_id = NULL,
    legacy_stripe_subscription_status = NULL,
    updated_at_ms = ?2,
    legacy_user_account_revision = legacy_user_account_revision + 1,
    legacy_user_account_last_operation_id = ?3
WHERE id = ?1;
