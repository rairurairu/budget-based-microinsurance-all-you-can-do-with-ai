variable "name" {
  description = "Prefix for every resource, such as microinsurance-tw-pilot."
  type        = string
}

variable "cidr" {
  description = "The VPC's address range."
  type        = string
  default     = "10.40.0.0/16"
}

variable "az_count" {
  description = "Availability zones used: two at least, for the Multi-AZ database."
  type        = number
  default     = 2
  validation {
    condition     = var.az_count >= 2
    error_message = "The database runs Multi-AZ: two zones at least."
  }
}

variable "nat_per_az" {
  description = "One NAT gateway per zone (production) or one for all (a pilot)."
  type        = bool
  default     = true
}

variable "api_port" {
  description = "The port the API listens on in its container."
  type        = number
  default     = 8080
}

variable "cloudfront_prefix_list_id" {
  description = "The managed prefix list of CloudFront's origin-facing addresses: only CloudFront reaches the load balancer."
  type        = string
}

variable "tags" {
  type    = map(string)
  default = {}
}
