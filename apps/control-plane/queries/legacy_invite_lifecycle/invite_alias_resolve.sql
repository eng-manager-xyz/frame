UPDATE legacy_invite_lifecycle_invite_aliases_v1
SET decision = ?2,
    resolved_at_ms = ?3,
    last_operation_id = ?4
WHERE mapped_invite_id = ?1
  AND decision = 'pending';
