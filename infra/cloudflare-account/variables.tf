variable "cloudflare_account_id" {
  description = "Cloudflare account that owns Frame R2 resources."
  type        = string
  sensitive   = true

  validation {
    condition     = can(regex("^[0-9a-f]{32}$", var.cloudflare_account_id))
    error_message = "cloudflare_account_id must be a 32-character hexadecimal account ID."
  }
}
variable "environment" {
  description = "Isolated Frame resource environment."
  type        = string

  validation {
    condition     = contains(["staging", "production"], var.environment)
    error_message = "environment must be staging or production."
  }
}

variable "bucket_name" {
  description = "Private canonical recordings bucket."
  type        = string

  validation {
    condition     = can(regex("^frame-recordings(-[a-z0-9-]+)?$", var.bucket_name))
    error_message = "bucket_name must be an explicit Frame recordings bucket."
  }
}

variable "location" {
  description = "Best-effort R2 location hint selected before bucket creation."
  type        = string
  default     = "wnam"

  validation {
    condition     = contains(["apac", "eeur", "enam", "weur", "wnam", "oc"], var.location)
    error_message = "location must be a supported R2 location hint."
  }
}

variable "allowed_browser_origins" {
  description = "Exact Frame origins allowed to use method-bound direct R2 capabilities."
  type        = list(string)

  validation {
    condition = length(var.allowed_browser_origins) > 0 && alltrue([
      for origin in var.allowed_browser_origins :
      can(regex("^https://[a-z0-9.-]+$", origin)) &&
      !endswith(origin, ".invalid") &&
      !strcontains(origin, "*")
    ])
    error_message = "allowed_browser_origins must contain exact HTTPS origins without paths, wildcards, or .invalid sentinels."
  }
}
