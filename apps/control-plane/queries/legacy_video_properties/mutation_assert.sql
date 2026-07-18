INSERT INTO legacy_video_property_assertions_v1 (
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1, COUNT(*)
FROM videos v
WHERE v.id = ?2
  AND v.title = ?3
  AND v.legacy_metadata_json IS ?4
  AND v.legacy_public = ?5
  AND v.legacy_password_hash IS ?6
  AND v.legacy_settings_json IS ?7
  AND v.updated_at_ms = ?8
  AND v.revision = ?9
  AND v.legacy_property_revision = ?10
  AND v.legacy_property_last_operation_id = ?1;
