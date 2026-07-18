UPDATE users
SET preferences_json = json_set(
      COALESCE(preferences_json, '{}'),
      '$.trackedEvents.user_signed_up',
      json('true')
    ),
    updated_at_ms = ?2
WHERE id = ?1 AND deleted_at_ms IS NULL
  AND ?2 - created_at_ms <= 604800000
  AND COALESCE(json_extract(
    preferences_json, '$.trackedEvents.user_signed_up'
  ), 0) <> 1
