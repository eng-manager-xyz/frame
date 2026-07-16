UPDATE business_legal_holds_v1
SET released_at_ms=?3
WHERE id=?1 AND organization_id=?2 AND released_at_ms IS NULL AND placed_at_ms<=?3
