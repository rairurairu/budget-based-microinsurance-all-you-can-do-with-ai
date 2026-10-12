variable "name" {
  type = string
}

variable "subnet_ids" {
  description = "Isolated subnets, one per zone."
  type        = list(string)
}

variable "security_group_id" {
  type = string
}

variable "kms_key_arn" {
  description = "The market's key: storage, snapshots, the master password and Performance Insights."
  type        = string
}

variable "instance_class" {
  description = "Sized from the load test against docs/performance.md (ledger postings, the policy store)."
  type        = string
  default     = "db.m7g.large"
}

variable "storage_gb" {
  type    = number
  default = 100
}

variable "max_storage_gb" {
  description = "Storage grows on its own up to this."
  type        = number
  default     = 1000
}

variable "backup_window" {
  description = "UTC; outside market business hours."
  type        = string
  default     = "18:00-19:00"
}

variable "maintenance_window" {
  description = "UTC; outside market business hours and the typhoon season's peak polls."
  type        = string
  default     = "sun:19:30-sun:20:30"
}

variable "log_slower_than_ms" {
  description = "Statements slower than this are logged."
  type        = number
  default     = 200
}

variable "tags" {
  type    = map(string)
  default = {}
}
