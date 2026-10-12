output "vpc_id" {
  value = aws_vpc.this.id
}

output "public_subnet_ids" {
  value = aws_subnet.public[*].id
}

output "private_subnet_ids" {
  value = aws_subnet.private[*].id
}

output "database_subnet_ids" {
  value = aws_subnet.database[*].id
}

output "security_group_ids" {
  value = {
    alb      = aws_security_group.alb.id
    api      = aws_security_group.api.id
    workers  = aws_security_group.workers.id
    database = aws_security_group.database.id
  }
}
