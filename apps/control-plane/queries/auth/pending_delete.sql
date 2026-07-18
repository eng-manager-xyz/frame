DELETE FROM auth_pending_verifications_v2
WHERE delivery_id = ?1 AND revision = ?2
