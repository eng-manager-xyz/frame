UPDATE shared_videos SET revoked_at_ms=?3, revision=revision+1, last_operation_id=?4
WHERE id=?1 AND organization_id=?2 AND revoked_at_ms IS NULL AND shared_at_ms<=?3
