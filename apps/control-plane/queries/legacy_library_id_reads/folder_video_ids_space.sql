SELECT video_alias.legacy_video_id AS legacy_video_id
FROM legacy_library_space_aliases_v1 space_alias
JOIN folders folder
  ON folder.legacy_scope_kind = 'space'
 AND folder.legacy_scope_id = space_alias.space_id
 AND folder.organization_id = ?3
 AND folder.legacy_folder_id = ?1
 AND folder.deleted_at_ms IS NULL
JOIN space_videos placement
  ON placement.space_id = space_alias.space_id
 AND placement.folder_id = folder.id
LEFT JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = placement.video_id
WHERE space_alias.legacy_space_id = ?2;
