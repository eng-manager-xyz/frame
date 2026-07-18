SELECT video_alias.legacy_video_id AS legacy_video_id
FROM legacy_user_account_organization_ids_v1 organization_alias
JOIN shared_videos placement
  ON placement.organization_id = organization_alias.organization_id
 AND placement.folder_id IS NULL
 AND placement.revoked_at_ms IS NULL
LEFT JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = placement.video_id
WHERE organization_alias.legacy_organization_id = ?1
  AND organization_alias.organization_id = ?2;
