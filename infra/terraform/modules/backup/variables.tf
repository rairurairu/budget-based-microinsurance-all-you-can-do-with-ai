variable "name" {
  type = string
}

variable "kms_key_arn" {
  type = string
}

variable "resource_arns" {
  description = "What is backed up: the database."
  type        = list(string)
}

variable "record_retention_years" {
  description = "`privacy.record_retention_years` of the insurer profile (R54)."
  type        = number
  default     = 7
}

variable "lock_changeable_for_days" {
  description = "Days before the vault lock can no longer be changed or removed."
  type        = number
  default     = 3
}

variable "tags" {
  type    = map(string)
  default = {}
}
