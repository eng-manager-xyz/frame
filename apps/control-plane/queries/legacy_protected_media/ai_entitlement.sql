SELECT entitlement.user_id,entitlement.entitlement_revision,
       entitlement.expires_at_ms
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id=alias.mapped_video_id
JOIN legacy_protected_media_ai_entitlements_v1 entitlement
  ON entitlement.user_id=video.owner_id
WHERE alias.legacy_video_id=?1
  AND video.deleted_at_ms IS NULL
  AND entitlement.state='active'
  AND COALESCE(entitlement.expires_at_ms,9007199254740991)>?2
LIMIT 2;
