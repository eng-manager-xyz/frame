UPDATE object_deletion_jobs
SET state = 'deleted', updated_at_ms = ?2
WHERE storage_object_id IN (
  SELECT id FROM storage_objects WHERE last_operation_id = ?1
);
