INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_oauth_reservations_v2 reservation
         WHERE reservation.id = ?1
       ) THEN 1 ELSE 0 END
