INSERT INTO legacy_extension_instant_assertions_v1(operation_id, assertion_kind, accepted)
SELECT ?1, 'progress_authority', CASE WHEN COUNT(*) = 1 THEN 1 ELSE 0 END
FROM legacy_extension_instant_recordings_v1 instant
JOIN videos video ON video.id = instant.mapped_video_id
WHERE instant.legacy_video_id = ?2
  AND instant.mapped_video_id = ?3
  AND instant.actor_id = ?4
  AND instant.organization_id = ?5
  AND instant.lifecycle_state = 'active'
  AND video.owner_id = ?4
  AND video.organization_id = ?5
  AND video.deleted_at_ms IS NULL;
