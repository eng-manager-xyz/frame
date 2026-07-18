UPDATE legacy_invite_lifecycle_member_aliases_v1
SET removed_at_ms = ?2, removed_operation_id = ?3
WHERE legacy_member_id = ?1 AND removed_at_ms IS NULL
