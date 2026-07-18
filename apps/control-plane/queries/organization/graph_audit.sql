SELECT finding_kind, subject_id, observed_revision
FROM (
  SELECT 'missing_owner_membership' AS finding_kind,
         o.owner_id AS subject_id,
         o.revision AS observed_revision
  FROM organizations o
  LEFT JOIN organization_members m
    ON m.organization_id = o.id AND m.user_id = o.owner_id
   AND m.role = 'owner' AND m.state = 'active'
  WHERE o.id = ?1 AND m.user_id IS NULL

  UNION ALL
  SELECT 'multiple_active_owners', o.id, o.revision
  FROM organizations o JOIN organization_members m ON m.organization_id = o.id
  WHERE o.id = ?1 AND m.role = 'owner' AND m.state = 'active'
  GROUP BY o.id HAVING COUNT(*) <> 1

  UNION ALL
  SELECT 'owner_pointer_mismatch', m.user_id, m.revision
  FROM organizations o JOIN organization_members m ON m.organization_id = o.id
  WHERE o.id = ?1 AND m.role = 'owner' AND m.state = 'active' AND m.user_id <> o.owner_id

  UNION ALL
  SELECT 'membership_without_user', m.user_id, m.revision
  FROM organization_members m LEFT JOIN users u ON u.id = m.user_id
  WHERE m.organization_id = ?1 AND u.id IS NULL
)
ORDER BY finding_kind, subject_id
LIMIT ?2
