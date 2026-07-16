SELECT CASE WHEN
  (?3 IS NULL OR NOT EXISTS (
    SELECT 1 FROM auth_session_mutation_grants_v2 g WHERE g.id = ?3
  ))
  AND EXISTS (
    SELECT 1 FROM auth_audit_events_v2 a
    WHERE a.operation_id = ?2
      AND a.action = 'verification_issue'
      AND a.outcome = 'allow'
      AND a.reason = 'verification_accepted'
  )
  AND (
    EXISTS (
      SELECT 1
      FROM auth_pending_verifications_v2 p
      WHERE p.delivery_id = ?1
        AND p.last_operation_id = ?2
    )
    OR (
      NOT EXISTS (
        SELECT 1 FROM auth_pending_verifications_v2 p WHERE p.delivery_id = ?1
      )
      AND (
        SELECT COUNT(*)
        FROM auth_verification_challenges_v2 c
        WHERE c.identifier_key_version = ?4
          AND c.identifier_digest = ?5
          AND c.secret_key_version = ?6
          AND c.secret_digest = ?7
          AND c.purpose = ?8
          AND c.channel = ?9
          AND c.max_attempts = ?10
          AND c.created_at_ms = ?11
          AND c.expires_at_ms = ?12
      ) = 1
      AND (
        (
          SELECT COUNT(*)
          FROM auth_delivery_outbox_v2 d
          WHERE d.delivery_id = ?1
            AND d.sealed_payload_hex = ?13
            AND d.created_at_ms = ?11
            AND d.expires_at_ms = ?12
        ) = 1
        OR (
          SELECT COUNT(*)
          FROM auth_delivery_ack_tombstones_v2 t
          WHERE t.delivery_id = ?1
        ) = 1
      )
    )
  )
THEN 1 ELSE 0 END AS present
