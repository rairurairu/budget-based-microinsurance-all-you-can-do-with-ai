variable "name" {
  type = string
}

variable "origin_domain_name" {
  description = "The load balancer's name on the market's own domain (its certificate's name)."
  type        = string
}

variable "aliases" {
  description = "The public names, such as cover.insurer.example."
  type        = list(string)
}

variable "certificate_arn" {
  description = "The public names' certificate, in us-east-1 (a CloudFront requirement)."
  type        = string
}

variable "price_class" {
  description = "Edge locations used: PriceClass_200 includes Hong Kong and Taiwan."
  type        = string
  default     = "PriceClass_200"
}

variable "requests_per_5_minutes" {
  description = "Requests one address may make in five minutes before WAF blocks it."
  type        = number
  default     = 2000
}

variable "tags" {
  type    = map(string)
  default = {}
}
