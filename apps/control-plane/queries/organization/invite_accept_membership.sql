INSERT INTO organization_members(
  organization_id, user_id, role, state, has_pro_seat,
  created_at_ms, updated_at_ms, revision, authority_version, last_operation_id
)
SELECT organization_id, ?2, role, 'active', 0, ?3, ?3, 0, 0, ?4
FROM organization_invites
WHERE id = ?1
  AND status = 'accepted'
  AND accepted_by_user_id = ?2
  AND last_operation_id = ?4
  AND role <> 'owner'
