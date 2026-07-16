SELECT id, data_class, subject_id, reason_code, placed_by_user_id,
       placed_at_ms, released_at_ms
FROM business_legal_holds_v1
WHERE organization_id=?1
ORDER BY placed_at_ms DESC, id DESC
LIMIT 1000
