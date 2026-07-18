INSERT INTO business_legal_holds_v1(
  id, organization_id, data_class, subject_id, reason_code,
  placed_by_user_id, placed_at_ms, released_at_ms
) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)
