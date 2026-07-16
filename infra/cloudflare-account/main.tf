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
        "content-length",
        "content-type",
        "if-match",
        "if-none-match",
        "range",
        "x-amz-checksum-sha256",
        "x-amz-meta-frame-sha256",
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

  # The provider lifecycle aborts abandoned multipart state. The Worker uses
  # an expiry receipt to delete browser-direct staging; published objects and
  # user data remain manifest/hold/deletion-workflow authority.
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
