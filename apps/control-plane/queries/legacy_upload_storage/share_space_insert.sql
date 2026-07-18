INSERT INTO legacy_upload_storage_space_shares_v1(
  mapped_video_id, space_id, shared_by_user_id, shared_at_ms, last_operation_id
)
SELECT ?1, space.id, ?2, ?4, ?5
FROM json_each(?3) selected
JOIN legacy_library_space_aliases_v1 alias ON alias.legacy_space_id = selected.value
JOIN spaces space ON space.id = alias.space_id AND space.deleted_at_ms IS NULL
JOIN organizations organization
  ON organization.id = space.organization_id AND organization.status = 'active'
WHERE organization.owner_id = ?2 OR EXISTS (
  SELECT 1 FROM organization_members member
  WHERE member.organization_id = organization.id
    AND member.user_id = ?2 AND member.state = 'active'
)
GROUP BY space.id;
