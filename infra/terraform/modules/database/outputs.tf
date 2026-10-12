output "arn" {
  value = aws_db_instance.this.arn
}

output "identifier" {
  value = aws_db_instance.this.identifier
}

output "endpoint" {
  description = "host:port, for the application's DATABASE_URL."
  value       = aws_db_instance.this.endpoint
}

output "master_user_secret_arn" {
  description = "The master password, kept by RDS in Secrets Manager."
  value       = aws_db_instance.this.master_user_secret[0].secret_arn
}
