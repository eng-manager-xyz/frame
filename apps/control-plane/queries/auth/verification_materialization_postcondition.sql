SELECT CASE WHEN
  NOT EXISTS (
    SELECT 1 FROM auth_pending_verifications_v2 p WHERE p.delivery_id = ?1
  )
  AND (
    ?2 = 'expired_deleted'
    OR (
      ?2 = 'materialized'
      AND EXISTS (
        SELECT 1 FROM auth_verification_challenges_v2 v
        WHERE v.id = ?3 AND v.last_operation_id = ?4
      )
      AND EXISTS (
        SELECT 1 FROM auth_delivery_outbox_v2 d
        WHERE d.delivery_id = ?1
          AND d.suppress = ?5
          AND d.last_operation_id = ?4
      )
    )
  )
THEN 1 ELSE 0 END AS present
