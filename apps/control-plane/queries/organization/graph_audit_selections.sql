SELECT finding_kind, subject_id, observed_revision
FROM (
  SELECT 'active_selection_without_membership' AS finding_kind,
         u.id AS subject_id,
         u.organization_preference_revision AS observed_revision
  FROM users u LEFT JOIN organization_members m
    ON m.organization_id = u.active_organization_id AND m.user_id = u.id AND m.state = 'active'
  WHERE u.active_organization_id = ?1 AND m.user_id IS NULL

  UNION ALL
  SELECT 'space_membership_without_organization_membership', sm.user_id, sm.revision
  FROM space_members sm JOIN spaces s ON s.id = sm.space_id
  LEFT JOIN organization_members m
    ON m.organization_id = s.organization_id AND m.user_id = sm.user_id AND m.state = 'active'
  WHERE s.organization_id = ?1 AND sm.state = 'active' AND m.user_id IS NULL
)
ORDER BY finding_kind, subject_id
LIMIT ?2
