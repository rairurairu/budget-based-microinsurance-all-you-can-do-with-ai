variable "name" {
  type = string
}

variable "kms_key_arn" {
  type = string
}

variable "alarm_emails" {
  description = "Who is told: the on-call addresses."
  type        = list(string)
  default     = []
}

variable "alb_arn_suffix" {
  type = string
}

variable "target_group_arn_suffix" {
  type = string
}

variable "db_identifier" {
  type = string
}

variable "p99_seconds" {
  description = "API p99 alarm, in seconds: AI routes wait up to the harness timeout (4 s)."
  type        = number
  default     = 4.5
}

variable "min_free_storage_gb" {
  type    = number
  default = 20
}

variable "tags" {
  type    = map(string)
  default = {}
}
