output "arns" {
  description = "Each secret's ARN, by name."
  value       = { for k, s in aws_secretsmanager_secret.this : k => s.arn }
}
