INSERT INTO legacy_extension_instant_assertions_v1(operation_id, assertion_kind, accepted)
SELECT ?1, 'create_postcondition', CASE WHEN COUNT(*) = 1 THEN 1 ELSE 0 END
FROM legacy_extension_instant_recordings_v1 instant
JOIN legacy_collaboration_video_aliases_v1 alias
  ON alias.legacy_video_id = instant.legacy_video_id
 AND alias.mapped_video_id = instant.mapped_video_id
JOIN videos video ON video.id = instant.mapped_video_id
WHERE instant.legacy_video_id = ?2
  AND instant.mapped_video_id = ?3
  AND instant.actor_id = ?4
  AND instant.organization_id = ?5
  AND instant.source_object_key = ?6
  AND instant.lifecycle_state = 'active'
  AND video.owner_id = instant.actor_id
  AND video.organization_id = instant.organization_id
  AND video.deleted_at_ms IS NULL
  AND ((?7 = 1 AND instant.upload_id = ?8) OR (?7 = 0 AND instant.upload_id IS NULL));
