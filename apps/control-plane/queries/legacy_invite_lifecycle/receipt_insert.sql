INSERT INTO legacy_invite_lifecycle_receipts_v1(
  operation_id, action, membership_existed, membership_created,
  membership_removed, pro_seat_assigned, inherited_subscription_cleared,
  fallback_organization_id, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9);
