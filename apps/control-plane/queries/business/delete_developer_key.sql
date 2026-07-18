DELETE FROM developer_api_keys
WHERE id=?1 AND EXISTS (
  SELECT 1 FROM developer_apps app WHERE app.id=developer_api_keys.app_id AND app.organization_id=?2
) AND ?3>=0 AND length(?4)=36
