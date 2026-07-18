WITH
actor AS (
  SELECT id,email,status,deleted_at_ms,active_organization_id,default_organization_id,
         legacy_user_account_authority_version AS actor_revision,
         legacy_third_party_stripe_subscription_id,
         legacy_stripe_subscription_id,legacy_stripe_subscription_status,
         legacy_stripe_customer_id
  FROM users WHERE id=?1
),
target AS (
  SELECT CASE
    WHEN ?7='video' AND ?5 IS NOT NULL THEN (
      SELECT mapped_video_id FROM legacy_collaboration_video_aliases_v1
      WHERE legacy_video_id=?5
    )
    WHEN ?7='space' AND ?5 IS NOT NULL THEN (
      SELECT space_id FROM legacy_library_space_aliases_v1
      WHERE legacy_space_id=?5
    )
    WHEN ?7='external' THEN ?5
    ELSE NULL
  END AS target_id
),
scope AS (
  SELECT CASE
    WHEN ?15='cap-v1-60f863b2cb19353f' AND (SELECT target_id FROM target) IS NOT NULL
      THEN (SELECT organization_id FROM videos
        WHERE id=(SELECT target_id FROM target) AND deleted_at_ms IS NULL)
    WHEN ?15='cap-v1-60f863b2cb19353f' AND ?4 IS NOT NULL THEN (
      SELECT organization_id FROM legacy_user_account_organization_ids_v1
      WHERE legacy_organization_id=?4
    )
    WHEN ?15='cap-v1-60f863b2cb19353f' THEN COALESCE(
      (SELECT organization.id FROM organizations organization
       LEFT JOIN organization_members member
         ON member.organization_id=organization.id AND member.user_id=?1
        AND member.state='active'
       WHERE organization.id=(SELECT default_organization_id FROM actor)
         AND organization.status='active'
         AND (organization.owner_id=?1 OR member.user_id IS NOT NULL)),
      (SELECT organization.id FROM organizations organization
       LEFT JOIN organization_members member
         ON member.organization_id=organization.id AND member.user_id=?1
        AND member.state='active'
       WHERE organization.status='active'
         AND (organization.owner_id=?1 OR member.user_id IS NOT NULL)
       ORDER BY organization.created_at_ms,organization.id LIMIT 1)
    )
    WHEN ?6='organization' AND ?4 IS NOT NULL THEN (
      SELECT organization_id FROM legacy_user_account_organization_ids_v1
      WHERE legacy_organization_id=?4
    )
    WHEN ?6='external' THEN ?4
    ELSE COALESCE(?3,(SELECT active_organization_id FROM actor))
  END AS tenant_id
),
credential AS (
  SELECT CASE
    WHEN ?8='none' AND ?9 IS NULL AND ?10 IS NULL AND ?11 IS NULL AND ?12 IS NULL
      THEN 9007199254740991
    WHEN ?8='session_token' THEN (
      SELECT MIN(session.idle_expires_at_ms,session.absolute_expires_at_ms)
      FROM auth_sessions_v2 session
      JOIN auth_identities_v2 identity ON identity.user_id=session.user_id
      JOIN users user ON user.id=session.user_id
      WHERE session.id=?9 AND session.user_id=?1
        AND session.token_key_version=?10 AND session.token_digest=?11
        AND session.state='active' AND session.revoked_at_ms IS NULL
        AND session.session_version=identity.session_version
        AND user.status='active' AND user.deleted_at_ms IS NULL
    )
    WHEN ?8='api_key' AND ?10 IS NULL THEN (
      SELECT COALESCE(key.expires_at_ms,9007199254740991)
      FROM auth_api_keys key JOIN users user ON user.id=key.user_id
      WHERE key.id=?9 AND key.user_id=?1 AND key.key_digest=?11
        AND key.revoked_at_ms IS NULL
        AND user.status='active' AND user.deleted_at_ms IS NULL
    )
    WHEN ?8='signed_state' AND ?9='google-drive-oauth-state.v1'
      AND ?10=1 AND ?11 IS NOT NULL THEN ?12
    WHEN ?8='signed_endpoint' AND ?9='media-server-webhook.endpoint.v1' THEN (
      SELECT COALESCE(authority.expires_at_ms,9007199254740991)
      FROM legacy_protected_integration_signed_authorities_v1 authority
      WHERE authority.credential_subject_id=?9
        AND authority.credential_kind=?8
        AND authority.credential_key_version=?10
        AND authority.credential_digest=?11
        AND authority.state='active'
    )
  END AS expires_at_ms
),
owning_organization AS (
  SELECT organization.id,organization.owner_id,organization.authority_version
  FROM organizations organization
  WHERE organization.id=CASE
      WHEN ?7='space' THEN (
        SELECT space.organization_id FROM spaces space
        WHERE space.id=(SELECT target_id FROM target) AND space.deleted_at_ms IS NULL
      )
      WHEN ?6='organization' OR ?15 IN (
        'cap-v1-0c233c1115838206','cap-v1-5e7e4265d65c8365'
      ) THEN (SELECT tenant_id FROM scope)
    END
    AND organization.status='active'
),
workflow_parent AS (
  SELECT parent.source_operation_id
  FROM legacy_protected_effect_parent_registry_v1 parent
  JOIN legacy_protected_effect_parent_edges_v1 edge
    ON edge.parent_family=parent.parent_family
   AND edge.parent_operation_id=parent.source_operation_id
   AND edge.child_family='protected_integrations'
   AND edge.child_operation_id=?15
  WHERE parent.parent_family=?19
    AND parent.parent_receipt_id=?20
    AND parent.request_digest=?21
    AND parent.authority_binding_digest=?22
    AND parent.state<>'dead_letter'
),
facts AS (
  SELECT
    (SELECT tenant_id FROM scope) AS tenant_id,
    (SELECT target_id FROM target) AS target_id,
    (SELECT expires_at_ms FROM credential) AS expires_at_ms,
    (SELECT actor_revision FROM actor) AS actor_revision,
    CASE WHEN ?6='organization' OR ?15 IN (
      'cap-v1-0c233c1115838206','cap-v1-5e7e4265d65c8365'
    ) THEN (
      SELECT authority_version FROM organizations
      WHERE id=(SELECT tenant_id FROM scope) AND status='active'
    ) ELSE 0 END AS scope_revision,
    CASE
      WHEN ?7='video' THEN (
        SELECT legacy_property_revision FROM videos
        WHERE id=(SELECT target_id FROM target) AND deleted_at_ms IS NULL
      )
      WHEN ?7='space' THEN (
        SELECT authority_version FROM spaces
        WHERE id=(SELECT target_id FROM target) AND deleted_at_ms IS NULL
      )
      ELSE 0
    END AS resource_revision,
    CASE WHEN ?7='space' THEN (
      SELECT is_public FROM spaces
      WHERE id=(SELECT target_id FROM target) AND deleted_at_ms IS NULL
    ) END AS current_public,
    (SELECT owner_id FROM owning_organization) AS owner_id,
    (SELECT owner.legacy_user_account_authority_version
      FROM users owner
      WHERE owner.id=(SELECT owner_id FROM owning_organization)
        AND owner.status='active' AND owner.deleted_at_ms IS NULL
    ) AS owner_revision
),
base AS (
  SELECT
    facts.*,
    CASE
      WHEN facts.expires_at_ms IS NULL OR facts.expires_at_ms<=?16 THEN 0
      WHEN ?6='organization' AND ?4 IS NOT NULL AND facts.tenant_id IS NULL THEN 0
      WHEN ?7 IN ('video','space') AND ?5 IS NOT NULL AND facts.target_id IS NULL
        AND ?15<>'cap-v1-60f863b2cb19353f' THEN 0
      WHEN ?15='cap-v1-f0a00e93ab606a52' AND NOT EXISTS (
        SELECT 1 FROM legacy_collaboration_user_aliases_v1 loom_actor
        JOIN legacy_user_account_organization_ids_v1 cap_organization
          ON cap_organization.legacy_organization_id=?4
        JOIN legacy_user_account_organization_ids_v1 loom_organization
          ON loom_organization.legacy_organization_id=?18
        WHERE loom_actor.legacy_user_id=?17
          AND loom_actor.mapped_user_id=?1
          AND cap_organization.organization_id=facts.tenant_id
          AND loom_organization.organization_id=facts.tenant_id
          AND facts.tenant_id=?3
      ) THEN 0
      WHEN ?15='cap-v1-b9fcb0fbd25b2234' AND NOT EXISTS (
        SELECT 1 FROM videos video
        JOIN legacy_collaboration_user_aliases_v1 owner_alias
          ON owner_alias.mapped_user_id=video.owner_id
         AND owner_alias.legacy_user_id=?17
        WHERE video.id=facts.target_id AND video.deleted_at_ms IS NULL
          AND facts.tenant_id IS NOT NULL
          AND video.organization_id=facts.tenant_id
          AND (
            ((SELECT source_operation_id FROM workflow_parent)='cap-v1-d062d262b013a0cd'
              AND ?23=?17||'/'||?5||'/raw-upload.mp4'
              AND EXISTS (
                SELECT 1 FROM organization_members member
                WHERE member.organization_id=facts.tenant_id
                  AND member.user_id=video.owner_id AND member.state='active'
              ))
            OR ((SELECT source_operation_id FROM workflow_parent)='cap-v1-cb3fade2af06d6bd'
              AND video.owner_id=?1
              AND ?23=?17||'/'||?5||'/raw-upload.mp4')
            OR ((SELECT source_operation_id FROM workflow_parent)='cap-v1-94a9944ce37fa085'
              AND video.owner_id=?1
              AND EXISTS (
                SELECT 1 FROM legacy_mobile_cap_uploads_v1 upload
                WHERE upload.mapped_video_id=video.id AND upload.raw_file_key=?23
              ))
          )
      ) THEN 0
      WHEN ?15='cap-v1-bd1b9d67380624f7' AND NOT EXISTS (
        SELECT 1 FROM legacy_collaboration_user_aliases_v1 actor_alias
        JOIN legacy_user_account_organization_ids_v1 cap_organization
          ON cap_organization.legacy_organization_id=?18
        JOIN legacy_user_account_organization_ids_v1 loom_organization
          ON loom_organization.legacy_organization_id=?4
        WHERE actor_alias.legacy_user_id=?17
          AND actor_alias.mapped_user_id=?1
          AND facts.tenant_id=?3
          AND cap_organization.organization_id=facts.tenant_id
          AND loom_organization.organization_id=facts.tenant_id
          AND (SELECT source_operation_id FROM workflow_parent)='cap-v1-f0a00e93ab606a52'
      ) THEN 0
      WHEN ?14 NOT IN ('none','cap_internal','pro','subscription_read','subscription_manage','ai_owner') THEN 0
      WHEN ?14='cap_internal' AND NOT EXISTS (
        SELECT 1 FROM actor WHERE lower(email) LIKE '%@cap.so'
      ) THEN 0
      WHEN ?14='pro' AND NOT EXISTS (
        SELECT 1 FROM actor WHERE legacy_third_party_stripe_subscription_id IS NOT NULL
          OR legacy_stripe_subscription_status IN ('active','trialing','complete','paid')
      ) THEN 0
      WHEN ?14='subscription_read' AND NOT EXISTS (
        SELECT 1 FROM actor WHERE legacy_stripe_subscription_id IS NOT NULL
      ) THEN 0
      WHEN ?14='subscription_manage' AND NOT EXISTS (
        SELECT 1 FROM actor WHERE legacy_stripe_subscription_id IS NOT NULL
          AND legacy_stripe_customer_id IS NOT NULL
      ) THEN 0
      WHEN ?14='ai_owner' AND NOT EXISTS (
        SELECT 1 FROM legacy_protected_media_ai_entitlements_v1 entitlement
        WHERE entitlement.user_id=?1 AND entitlement.state='active'
          AND COALESCE(entitlement.expires_at_ms,9007199254740991)>?16
      ) THEN 0
      WHEN ?2='public' THEN 1
      WHEN ?2='signed_webhook' THEN CASE WHEN EXISTS (
        SELECT 1 FROM legacy_protected_integration_signed_authorities_v1 authority
        WHERE authority.credential_subject_id=?9 AND authority.credential_kind=?8
          AND authority.credential_key_version=?10 AND authority.credential_digest=?11
          AND authority.state='active'
          AND COALESCE(authority.expires_at_ms,9007199254740991)>?16
      ) AND (
        facts.target_id IS NULL OR EXISTS (
          SELECT 1 FROM videos WHERE id=facts.target_id AND deleted_at_ms IS NULL
        )
      ) THEN 1 ELSE 0 END
      WHEN NOT EXISTS (
        SELECT 1 FROM actor WHERE status='active' AND deleted_at_ms IS NULL
      ) THEN 0
      WHEN ?2 IN ('session','signed_state') THEN 1
      WHEN ?2='session_or_organization_member' THEN CASE
        WHEN ?4 IS NULL THEN 1
        WHEN EXISTS (
          SELECT 1 FROM organizations organization
          LEFT JOIN organization_members member
            ON member.organization_id=organization.id AND member.user_id=?1
              AND member.state='active'
          WHERE organization.id=facts.tenant_id AND organization.status='active'
            AND (organization.owner_id=?1 OR member.user_id IS NOT NULL)
        ) THEN 1 ELSE 0 END
      WHEN ?2='session_or_organization_owner' THEN CASE
        WHEN ?4 IS NULL THEN 1
        WHEN EXISTS (
          SELECT 1 FROM organizations organization
          WHERE organization.id=facts.tenant_id AND organization.status='active'
            AND organization.owner_id=?1
        ) THEN 1 ELSE 0 END
      WHEN ?2='signed_state_or_organization_owner' THEN CASE
        WHEN ?4 IS NULL THEN 1
        WHEN EXISTS (
          SELECT 1 FROM organizations organization
          WHERE organization.id=facts.tenant_id AND organization.status='active'
            AND organization.owner_id=?1
        ) THEN 1 ELSE 0 END
      WHEN ?2='organization_member' THEN CASE
        WHEN ?15='cap-v1-60f863b2cb19353f' AND facts.target_id IS NOT NULL
          THEN CASE WHEN EXISTS (
            SELECT 1 FROM videos video
            WHERE video.id=facts.target_id AND video.owner_id=?1
              AND video.organization_id=facts.tenant_id
              AND video.deleted_at_ms IS NULL
          ) THEN 1 ELSE 0 END
        WHEN EXISTS (
          SELECT 1 FROM organizations organization
          LEFT JOIN organization_members member
            ON member.organization_id=organization.id AND member.user_id=?1
              AND member.state='active'
          WHERE organization.id=facts.tenant_id AND organization.status='active'
            AND (organization.owner_id=?1 OR member.user_id IS NOT NULL)
        ) THEN 1 ELSE 0 END
      WHEN ?2='organization_manager' THEN CASE WHEN EXISTS (
        SELECT 1 FROM organizations organization
        LEFT JOIN organization_members member
          ON member.organization_id=organization.id AND member.user_id=?1
            AND member.state='active'
        WHERE organization.id=facts.tenant_id AND organization.status='active'
          AND (organization.owner_id=?1 OR member.role IN ('owner','admin'))
      ) THEN 1 ELSE 0 END
      WHEN ?2='organization_owner' THEN CASE WHEN EXISTS (
        SELECT 1 FROM organizations organization
        WHERE organization.id=facts.tenant_id AND organization.status='active'
          AND organization.owner_id=?1
      ) THEN 1 ELSE 0 END
      WHEN ?2='space_manager' THEN CASE WHEN EXISTS (
        SELECT 1 FROM spaces space
        JOIN organizations organization ON organization.id=space.organization_id
        LEFT JOIN organization_members member
          ON member.organization_id=organization.id AND member.user_id=?1
            AND member.state='active'
        LEFT JOIN space_members space_member
          ON space_member.space_id=space.id AND space_member.user_id=?1
        WHERE space.id=facts.target_id AND space.deleted_at_ms IS NULL
          AND organization.status='active'
          AND (organization.owner_id=?1 OR member.role IN ('owner','admin')
            OR space_member.role='manager')
      ) THEN 1 ELSE 0 END
      WHEN ?2='video_viewer' THEN CASE WHEN EXISTS (
        SELECT 1 FROM videos video
        WHERE video.id=facts.target_id AND video.deleted_at_ms IS NULL
          AND json_valid(?13) AND json_type(?13)='array' AND json_array_length(?13)=1
          AND json_extract(?13,'$[0].target_id')=video.id
          AND (
            (video.owner_id=?1
              AND json_extract(?13,'$[0].kind')='owner_bypass'
              AND json_extract(?13,'$[0].subject_id')=video.id
              AND json_extract(?13,'$[0].revision')=video.legacy_property_revision)
            OR (
              (
                EXISTS (SELECT 1 FROM organization_members member
                  WHERE member.organization_id=video.organization_id
                    AND member.user_id=?1 AND member.state='active')
                OR EXISTS (SELECT 1 FROM space_videos placement
                  JOIN space_members member ON member.space_id=placement.space_id
                  JOIN spaces space ON space.id=placement.space_id
                  WHERE placement.video_id=video.id AND member.user_id=?1
                    AND space.deleted_at_ms IS NULL)
                OR (video.legacy_public=1 AND (
                  COALESCE(TRIM((SELECT legacy_allowed_email_restriction
                    FROM organizations WHERE id=video.organization_id)),'')=''
                  OR (?1 IS NOT NULL AND EXISTS (
                    SELECT 1 FROM actor
                    WHERE instr(','||lower(replace((SELECT legacy_allowed_email_restriction
                      FROM organizations WHERE id=video.organization_id),' ',''))||',',
                      ','||lower(email)||',')>0
                       OR instr(','||lower(replace((SELECT legacy_allowed_email_restriction
                      FROM organizations WHERE id=video.organization_id),' ',''))||',',
                      ','||lower(substr(email,instr(email,'@')+1))||',')>0
                  ))
                ))
              )
              AND (
                (video.legacy_password_hash IS NOT NULL
                  AND json_extract(?13,'$[0].kind')='video_password'
                  AND json_extract(?13,'$[0].subject_id')=video.id
                  AND json_extract(?13,'$[0].revision')=video.legacy_property_revision)
                OR (video.legacy_password_hash IS NULL AND EXISTS (
                  SELECT 1 FROM space_videos placement JOIN spaces space
                    ON space.id=placement.space_id
                  WHERE placement.video_id=video.id AND space.deleted_at_ms IS NULL
                    AND space.legacy_password_hash IS NOT NULL
                    AND space.id=(
                      SELECT MIN(candidate.id)
                      FROM space_videos candidate_placement
                      JOIN spaces candidate ON candidate.id=candidate_placement.space_id
                      WHERE candidate_placement.video_id=video.id
                        AND candidate.deleted_at_ms IS NULL
                        AND candidate.legacy_password_hash IS NOT NULL
                    )
                    AND json_extract(?13,'$[0].kind')='space_password'
                    AND json_extract(?13,'$[0].subject_id')=space.id
                    AND json_extract(?13,'$[0].revision')=space.legacy_password_revision
                ))
                OR (video.legacy_password_hash IS NULL AND NOT EXISTS (
                  SELECT 1 FROM space_videos placement JOIN spaces space
                    ON space.id=placement.space_id
                  WHERE placement.video_id=video.id AND space.deleted_at_ms IS NULL
                    AND space.legacy_password_hash IS NOT NULL
                ) AND json_extract(?13,'$[0].kind')='unprotected_video_policy'
                  AND json_extract(?13,'$[0].subject_id')=video.id
                  AND json_extract(?13,'$[0].revision')=video.legacy_property_revision)
              )
            )
          )
      ) THEN 1 ELSE 0 END
      WHEN ?2='parent_receipt' THEN CASE
        WHEN ?15 IN ('cap-v1-b9fcb0fbd25b2234','cap-v1-bd1b9d67380624f7')
          THEN CASE WHEN EXISTS (SELECT 1 FROM workflow_parent) THEN 1 ELSE 0 END
        ELSE 1 END
      ELSE 0
    END AS authorized
  FROM facts
)
SELECT authorized,resolved.tenant_id AS resolved_tenant_id,
       resolved.target_id AS resolved_target_id,
       resolved.expires_at_ms AS authority_expires_at_ms,
       resolved.actor_revision,resolved.scope_revision,resolved.resource_revision,
       resolved.current_public,resolved.owner_id,resolved.owner_revision
FROM base resolved;
