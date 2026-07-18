DELETE FROM auth_pending_verifications_v2
WHERE initiator_session_id IN (SELECT id FROM auth_sessions_v2 WHERE family_id = ?1)
