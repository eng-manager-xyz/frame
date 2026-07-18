UPDATE legacy_invite_lifecycle_member_aliases_v1
SET removed_at_ms = ?3,
    removed_operation_id = ?4
WHERE organization_id = ?1
  AND user_id = ?2
  AND removed_at_ms IS NULL
  AND ?5 = 1;
