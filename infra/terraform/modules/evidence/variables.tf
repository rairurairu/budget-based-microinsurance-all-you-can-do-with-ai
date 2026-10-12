variable "name" {
  description = "The bucket's globally unique name."
  type        = string
}

variable "kms_key_arn" {
  type = string
}

variable "noncurrent_days" {
  description = "Days an erased or replaced object's old version is kept before it expires."
  type        = number
  default     = 30
}

variable "tags" {
  type    = map(string)
  default = {}
}
