provider "cloudflare" {}

resource "cloudflare_r2_bucket" "recordings" {
  account_id    = var.cloudflare_account_id
  name          = var.bucket_name
  jurisdiction  = "default"
  location      = var.location
  storage_class = "Standard"

  lifecycle {
    prevent_destroy = true
  }
}
resource "cloudflare_r2_bucket_cors" "recordings" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.recordings.name

  rules = [{
    id = "frame-${var.environment}-direct-transfer"
    allowed = {
      origins = var.allowed_browser_origins
      methods = ["GET", "HEAD", "PUT"]
      headers = [
        "content-type",
        "if-match",
        "if-none-match",
        "range",
        "x-amz-checksum-sha256",
        "x-amz-content-sha256",
        "x-amz-date",
      ]
    }
    expose_headers = [
      "accept-ranges",
      "content-length",
      "content-range",
      "etag",
    ]
    max_age_seconds = 3600
  }]
}

resource "cloudflare_r2_bucket_lifecycle" "recordings" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.recordings.name

  # Only abandoned multipart state is cleaned automatically. Published objects
  # and user data are deleted by the manifest-driven hold/deletion workflow.
  rules = [{
    id      = "abort-abandoned-frame-uploads"
    enabled = true
    conditions = {
      prefix = "uploads/"
    }
    abort_multipart_uploads_transition = {
      condition = {
        max_age = 86400
        type    = "Age"
      }
    }
  }]
}
