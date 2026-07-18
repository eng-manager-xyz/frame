UPDATE legacy_developer_credit_accounts_v1
SET auto_top_up_enabled = ?4,
    auto_top_up_threshold_microcredits = CASE WHEN ?5 = 1 THEN ?6
      ELSE auto_top_up_threshold_microcredits END,
    auto_top_up_amount_cents = CASE WHEN ?7 = 1 THEN ?8
      ELSE auto_top_up_amount_cents END,
    updated_at_ms = ?9, revision = revision + 1, last_operation_id = ?1
WHERE app_id = ?2 AND owner_id = ?3 AND revision = ?10
  AND EXISTS (
    SELECT 1 FROM legacy_developer_apps_v1 app
    WHERE app.id = ?2 AND app.owner_id = ?3 AND app.deleted_at_ms IS NULL
      AND app.revision = ?11 AND app.authority_version = ?12
  )
