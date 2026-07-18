WITH actor AS (
  SELECT id,email,status,deleted_at_ms
  FROM users
  WHERE id = ?1
), credential AS (
  SELECT CASE
    WHEN ?4 = 'none' THEN CASE WHEN
      ?5 IS NULL AND ?6 IS NULL AND ?7 IS NULL
    THEN 1 ELSE 0 END
    WHEN ?4 = 'public_flow' THEN CASE WHEN
      ?5 IS NULL AND ?6 IS NULL
      AND ?7 IS NOT NULL AND length(?7) = 64
      AND ?7 NOT GLOB '*[^0-9a-f]*'
    THEN 1 ELSE 0 END
    WHEN ?4 = 'signed_endpoint' THEN CASE WHEN
      ?5 = 'stripe-webhook.endpoint.v1' AND ?6 IS NULL
      AND ?7 IS NOT NULL AND length(?7) = 64
      AND ?7 NOT GLOB '*[^0-9a-f]*'
    THEN 1 ELSE 0 END
    WHEN ?4 = 'session_token' THEN CASE WHEN EXISTS (
      SELECT 1
      FROM auth_sessions_v2 session
      JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
      WHERE session.id = ?5
        AND session.user_id = ?1
        AND session.token_key_version = ?6
        AND session.token_digest = ?7
        AND session.state = 'active'
        AND session.revoked_at_ms IS NULL
        AND session.session_version = identity.session_version
        AND session.idle_expires_at_ms > ?8
        AND session.absolute_expires_at_ms > ?8
    ) THEN 1 ELSE 0 END
    WHEN ?4 = 'api_key' THEN CASE WHEN
      ?6 IS NULL AND EXISTS (
        SELECT 1 FROM auth_api_keys key
        WHERE key.id = ?5
          AND key.user_id = ?1
          AND key.key_digest = ?7
          AND key.revoked_at_ms IS NULL
          AND (key.expires_at_ms IS NULL OR key.expires_at_ms > ?8)
      )
    THEN 1 ELSE 0 END
    ELSE 0
  END AS valid
)
SELECT CASE
  WHEN ?2 = 'public_flow' THEN CASE WHEN
    ?4 IN ('none','public_flow') AND (SELECT valid FROM credential) = 1
  THEN 1 ELSE 0 END
  WHEN ?2 = 'signed_stripe_webhook' THEN CASE WHEN
    ?4 = 'signed_endpoint' AND (SELECT valid FROM credential) = 1
  THEN 1 ELSE 0 END
  WHEN NOT EXISTS (
    SELECT 1 FROM actor WHERE status = 'active' AND deleted_at_ms IS NULL
  ) THEN 0
  WHEN ?2 = 'active_session' THEN CASE WHEN
    ?4 IN ('session_token','api_key') AND (SELECT valid FROM credential) = 1
  THEN 1 ELSE 0 END
  WHEN ?2 = 'developer_app_owner' THEN CASE WHEN
    ?4 IN ('session_token','api_key') AND (SELECT valid FROM credential) = 1
    AND EXISTS (
      SELECT 1
      FROM developer_apps app
      JOIN developer_credit_accounts account ON account.app_id = app.id
      WHERE app.id = ?3
        AND app.owner_user_id = ?1
        AND app.status = 'active'
        AND app.deleted_at_ms IS NULL
    )
  THEN 1 ELSE 0 END
  WHEN ?2 = 'messenger_admin_video' THEN CASE WHEN
    ?4 = 'session_token' AND (SELECT valid FROM credential) = 1
    AND EXISTS (
      SELECT 1
      FROM actor
      JOIN videos video ON video.id = ?3
      WHERE actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND actor.email = 'richie@cap.so' COLLATE NOCASE
        AND video.state <> 'deleted'
        AND video.deleted_at_ms IS NULL
    )
  THEN 1 ELSE 0 END
  ELSE 0
END AS authorized;
