UPDATE storage_objects SET state='deleted', deleted_at_ms=?3, updated_at_ms=?3,
  revision=revision+1, last_operation_id=?4
WHERE id=?1 AND organization_id=?2 AND state<>'deleted' AND created_at_ms<=?3
