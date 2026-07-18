DELETE FROM legacy_desktop_video_uploads_v1
WHERE video_id = ?1
  AND revision = ?2
  AND mode IS NOT 'multipart';
