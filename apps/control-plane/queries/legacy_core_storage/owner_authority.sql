SELECT
  media.mapped_video_id,
  media.legacy_video_id,
  media.owner_id,
  media.organization_id,
  media.object_prefix,
  media.source_type,
  integration.id AS storage_integration_id,
  COALESCE((
    SELECT owner_member.has_pro_seat
    FROM organization_members owner_member
    WHERE owner_member.organization_id = media.organization_id
      AND owner_member.user_id = organization.owner_id
      AND owner_member.state = 'active'
    LIMIT 1
  ), 0) AS organization_owner_has_pro_seat,
  COALESCE(json_extract(integration.capabilities_json, '$.single_put'), 0) AS supports_single_put,
  COALESCE(json_extract(integration.capabilities_json, '$.multipart'), 0) AS supports_multipart
FROM legacy_mobile_cap_media_v1 media
JOIN videos video
  ON video.id = media.mapped_video_id
 AND video.owner_id = media.owner_id
 AND video.organization_id = media.organization_id
JOIN organizations organization
  ON organization.id = media.organization_id
 AND organization.status = 'active'
JOIN organization_members member
  ON member.organization_id = media.organization_id
 AND member.user_id = ?1
 AND member.state = 'active'
JOIN storage_integrations integration
  ON integration.organization_id = media.organization_id
 AND integration.provider = 'r2'
 AND integration.state = 'active'
WHERE media.legacy_video_id = ?2
  AND media.owner_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
  AND (
    json_extract(integration.capabilities_json, '$.single_put') = 1
    OR json_extract(integration.capabilities_json, '$.multipart') = 1
  )
ORDER BY integration.updated_at_ms DESC, integration.id
LIMIT 2;
