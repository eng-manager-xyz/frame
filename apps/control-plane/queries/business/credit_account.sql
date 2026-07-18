SELECT account.id, account.app_id, account.balance_microcredits,
       account.auto_top_up_enabled, account.auto_top_up_threshold_microcredits,
       account.created_at_ms, account.updated_at_ms, account.revision,
       account.ledger_sequence
FROM developer_credit_accounts account
JOIN developer_apps app ON app.id=account.app_id
WHERE account.id=?1 AND app.organization_id=?2
