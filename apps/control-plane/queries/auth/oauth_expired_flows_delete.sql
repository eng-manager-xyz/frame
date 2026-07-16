DELETE FROM auth_oauth_flows_v2
WHERE expires_at_ms <= ?1
