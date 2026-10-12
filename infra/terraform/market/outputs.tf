output "public_domain_name" {
  description = "CloudFront's name: point the public names at it."
  value       = module.edge.domain_name
}

output "origin_load_balancer" {
  description = "Point `origin_domain_name` at it."
  value       = module.service.alb_dns_name
}

output "image_repository" {
  value = aws_ecr_repository.this.repository_url
}

output "database_endpoint" {
  value = module.database.endpoint
}

output "database_master_secret_arn" {
  value = module.database.master_user_secret_arn
}

output "secret_arns" {
  description = "Store each value out of band (README.md)."
  value       = module.secrets.arns
}

output "evidence_bucket" {
  value = module.evidence.bucket
}

output "alarm_topic_arn" {
  value = module.monitoring.alarm_topic_arn
}
