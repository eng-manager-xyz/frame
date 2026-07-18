SELECT video_alias.legacy_video_id AS legacy_video_id
FROM folders folder
JOIN shared_videos placement
  ON placement.folder_id = folder.id
 AND placement.revoked_at_ms IS NULL
LEFT JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = placement.video_id
WHERE folder.legacy_folder_id = ?1
  AND folder.organization_id = ?2
  AND folder.legacy_scope_kind = 'organization'
  AND folder.legacy_scope_id = ?2
  AND folder.deleted_at_ms IS NULL;
