DELETE FROM developer_app_domains
WHERE app_id || ':' || domain_ascii=?1 AND EXISTS (
  SELECT 1 FROM developer_apps app WHERE app.id=developer_app_domains.app_id AND app.organization_id=?2
) AND ?3>=0 AND length(?4)=36
