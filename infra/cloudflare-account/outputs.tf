output "recordings_bucket_name" {
  description = "Bucket name consumed by the matching Wrangler environment."
  value       = cloudflare_r2_bucket.recordings.name
}
output "cors_rule_id" {
  description = "Stable CORS rule identity for drift and release evidence."
  value       = "frame-${var.environment}-direct-transfer"
}
