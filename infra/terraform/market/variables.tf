variable "market" {
  description = "The market's code, such as hk or tw: names every resource."
  type        = string
}

variable "market_overlay" {
  description = "`MARKET` for the processes: a directory of config/markets/, or empty for the base market (ADR-012)."
  type        = string
}

variable "environment" {
  description = "pilot or production. A pilot shares one NAT gateway across zones."
  type        = string
  validation {
    condition     = contains(["pilot", "production"], var.environment)
    error_message = "pilot or production."
  }
}

variable "region" {
  description = "In or near the market, for data residency: ap-east-1 (Hong Kong), ap-east-2 (Taipei)."
  type        = string
}

variable "image" {
  description = "The release's image in this market's registry, by digest or immutable tag (linux/arm64)."
  type        = string
}

variable "vpc_cidr" {
  type    = string
  default = "10.40.0.0/16"
}

variable "az_count" {
  type    = number
  default = 2
}

variable "db_instance_class" {
  type    = string
  default = "db.m7g.large"
}

variable "api_min" {
  type    = number
  default = 2
}

variable "api_max" {
  type    = number
  default = 10
}

variable "workers" {
  type    = number
  default = 2
}

variable "origin_domain_name" {
  description = "The load balancer's own name, which CloudFront calls (a CNAME to it, outside this module)."
  type        = string
}

variable "origin_certificate_arn" {
  description = "ACM certificate for `origin_domain_name`, in the market's region."
  type        = string
}

variable "public_domain_names" {
  description = "The names customers, partners and staff use."
  type        = list(string)
}

variable "edge_certificate_arn" {
  description = "ACM certificate for `public_domain_names`, in us-east-1."
  type        = string
}

variable "record_retention_years" {
  description = "The insurer profile's `privacy.record_retention_years` (R54): monthly backups are kept this long."
  type        = number
  default     = 7
}

variable "alarm_emails" {
  type    = list(string)
  default = []
}

variable "tags" {
  type    = map(string)
  default = {}
}
