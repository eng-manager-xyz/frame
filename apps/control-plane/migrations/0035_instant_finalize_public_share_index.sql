-- Bound anonymous public-share projection to the retained finalize rows for
-- one tenant/video pair. The descending authority fields make the latest-row
-- choice deterministic, while the two projected fields keep both scalar
-- lookups covering without exposing provider or object identity.
CREATE INDEX instant_finalize_requests_v1_public_share_latest_idx
  ON instant_finalize_requests_v1(
    organization_id,
    video_id,
    updated_at_ms DESC,
    session_id DESC,
    state,
    last_failure_class
  );
