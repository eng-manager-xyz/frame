INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'mutation', 1,
  CASE WHEN
    (SELECT COUNT(*) FROM legacy_desktop_personal_storage_integrations_v1 row
     WHERE row.owner_user_id = ?2 AND row.active = 1) = CASE WHEN ?3 IS NULL THEN 0 ELSE 1 END
    AND (?3 IS NULL OR EXISTS (
      SELECT 1 FROM legacy_desktop_personal_storage_integrations_v1 selected
      WHERE selected.integration_id = ?3
        AND selected.owner_user_id = ?2
        AND selected.active = 1
        AND selected.last_operation_id = ?1
    ))
  THEN 1 ELSE 0 END;
