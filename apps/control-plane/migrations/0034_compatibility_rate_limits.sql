-- Durable, privacy-safe admission for every promoted exact compatibility
-- adapter. Subject values are keyed digests produced with the protected auth
-- hash-key ring; raw source addresses and principal identifiers never cross
-- this persistence boundary.
CREATE TABLE compatibility_rate_limit_buckets_v1 (
  bucket TEXT NOT NULL CHECK (bucket IN (
    'service_misc.v1',
    'client_compatibility.v1',
    'organization_library.v1',
    'collaboration_notifications.v1'
  )),
  dimension TEXT NOT NULL CHECK (dimension IN ('source', 'principal')),
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 1 AND 65535),
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64
    AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  window_started_at_ms INTEGER NOT NULL CHECK (
    window_started_at_ms BETWEEN 0 AND 9007199254740991
  ),
  request_count INTEGER NOT NULL CHECK (request_count BETWEEN 1 AND 1000000),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN 0 AND 9007199254740991
  ),
  gc_at_ms INTEGER NOT NULL CHECK (
    gc_at_ms BETWEEN 0 AND 9007199254740991
    AND gc_at_ms > updated_at_ms
  ),
  PRIMARY KEY (bucket, dimension, key_version, subject_digest)
);

CREATE INDEX compatibility_rate_limit_buckets_v1_gc_idx
  ON compatibility_rate_limit_buckets_v1(gc_at_ms);

-- Bounded state is part of the abuse-control contract. Once capacity is
-- exhausted a new subject fails closed; an existing subject can still be
-- updated, and each admission prunes a bounded number of expired rows.
CREATE TRIGGER compatibility_rate_limit_buckets_v1_cardinality_cap
BEFORE INSERT ON compatibility_rate_limit_buckets_v1
WHEN (SELECT COUNT(*) FROM compatibility_rate_limit_buckets_v1) >= 32768
  AND NOT EXISTS (
    SELECT 1
    FROM compatibility_rate_limit_buckets_v1 current
    WHERE current.bucket = NEW.bucket
      AND current.dimension = NEW.dimension
      AND current.key_version = NEW.key_version
      AND current.subject_digest = NEW.subject_digest
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_compatibility_rate_limit_capacity_v1');
END;
