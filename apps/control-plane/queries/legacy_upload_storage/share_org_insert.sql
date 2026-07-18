INSERT INTO legacy_upload_storage_organization_shares_v1(
  mapped_video_id, organization_id, shared_by_user_id, shared_at_ms,
  last_operation_id
)
SELECT ?1, organization.id, ?2, ?4, ?5
FROM json_each(?3) selected
JOIN legacy_user_account_organization_ids_v1 alias
  ON alias.legacy_organization_id = selected.value
JOIN organizations organization
  ON organization.id = alias.organization_id AND organization.status = 'active'
WHERE organization.owner_id = ?2 OR EXISTS (
  SELECT 1 FROM organization_members member
  WHERE member.organization_id = organization.id
    AND member.user_id = ?2 AND member.state = 'active'
)
GROUP BY organization.id;
