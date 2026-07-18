INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN
  ?10='adjustment' AND ?11='data_deletion_compensation'
  AND EXISTS (
    SELECT 1 FROM developer_credit_accounts account
    JOIN developer_apps app ON app.id=account.app_id
    WHERE account.id=?6 AND app.organization_id=?2
      AND ?7=account.ledger_sequence+1
      AND ?9=account.balance_microcredits+?8
      AND ?9 BETWEEN 0 AND 9007199254740991
  )
  AND (
    (?3='credit_transaction' AND EXISTS (
      SELECT 1 FROM developer_credit_transactions original
      JOIN developer_credit_accounts account ON account.id=original.account_id
      JOIN developer_apps app ON app.id=account.app_id
      WHERE original.id=?4 AND app.organization_id=?2
        AND original.account_id=?6 AND ?8=-original.amount_microcredits
    ))
    OR (?3='usage_ledger' AND EXISTS (
      SELECT 1 FROM usage_ledger usage
      LEFT JOIN developer_apps usage_app ON usage_app.id=usage.app_id
      JOIN developer_credit_accounts account ON account.id=?6
      JOIN developer_apps account_app ON account_app.id=account.app_id
      WHERE usage.id=?4 AND account_app.organization_id=?2
        AND (usage.organization_id=?2 OR usage_app.organization_id=?2)
        AND (usage.app_id IS NULL OR account.app_id=usage.app_id)
        AND ?8=usage.microcredits_charged
    ))
  )
  AND ?12=lower(?12) AND length(?12)=64 AND ?12 NOT GLOB '*[^0-9a-f]*'
THEN 1 ELSE 0 END
