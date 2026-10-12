variable "name" {
  type = string
}

variable "image" {
  description = "The release's image (infra/Dockerfile), by digest or immutable tag."
  type        = string
}

variable "market_overlay" {
  description = "`MARKET`: a directory of config/markets/, or empty for the base market (ADR-012)."
  type        = string
}

variable "insurer_profile" {
  description = "`INSURER_PROFILE`; empty for the market's own."
  type        = string
  default     = ""
}

variable "rating_basis_path" {
  description = "The filed rating basis in the image (RATING_BASIS)."
  type        = string
  default     = "/app/market-data/rating_basis.json"
}

variable "prevention_basis_path" {
  description = "The prevention basis in the image (PREVENTION_BASIS)."
  type        = string
  default     = "/app/market-data/prevention_basis.json"
}

variable "evidence_bucket" {
  type = string
}

variable "evidence_bucket_arn" {
  type = string
}

variable "secret_arns" {
  description = "api_token_secret, audit_seed, database_url, model_api_key."
  type        = map(string)
}

variable "kms_key_arn" {
  type = string
}

variable "vpc_id" {
  type = string
}

variable "public_subnet_ids" {
  type = list(string)
}

variable "private_subnet_ids" {
  type = list(string)
}

variable "security_group_ids" {
  description = "alb, api, workers."
  type        = map(string)
}

variable "certificate_arn" {
  description = "The load balancer's certificate (ACM, in the market's region)."
  type        = string
}

variable "api_port" {
  type    = number
  default = 8080
}

variable "api_min" {
  description = "API tasks at least: two, across zones."
  type        = number
  default     = 2
}

variable "api_max" {
  type    = number
  default = 10
}

variable "workers" {
  description = "Worker tasks: two, so a poll and the jobs survive one stopping."
  type        = number
  default     = 2
}

variable "task_cpu" {
  type    = map(number)
  default = { api = 1024, workers = 1024 }
}

variable "task_memory" {
  type    = map(number)
  default = { api = 2048, workers = 2048 }
}

variable "log_retention_days" {
  type    = number
  default = 400
}

variable "tags" {
  type    = map(string)
  default = {}
}
