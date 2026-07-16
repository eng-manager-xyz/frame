INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?4,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_oauth_reservations_v2 reservation
         WHERE reservation.id = ?1
           AND reservation.revision = ?2
           AND reservation.consumed_at_ms IS ?3
       ) THEN 1 ELSE 0 END
