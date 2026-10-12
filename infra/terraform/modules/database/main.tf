# The system of record: one PostgreSQL 16 database holding the policy store, the ledger, the job
# queue and the workers' state (ADR-001, ADR-011, ADR-012). Multi-AZ, encrypted with the market's
# key, TLS only, in subnets with no route out. The master password is generated and kept by RDS in
# Secrets Manager, never in Terraform state; the application connects as its own role, whose URL
# the operator stores in the `database_url` secret (README.md).

resource "aws_db_subnet_group" "this" {
  name       = var.name
  subnet_ids = var.subnet_ids
  tags       = var.tags
}

resource "aws_db_parameter_group" "this" {
  name   = var.name
  family = "postgres16"
  parameter {
    name  = "rds.force_ssl"
    value = "1"
  }
  parameter {
    name  = "log_min_duration_statement"
    value = tostring(var.log_slower_than_ms)
  }
  tags = var.tags
}

resource "aws_db_instance" "this" {
  identifier                            = var.name
  engine                                = "postgres"
  engine_version                        = "16"
  instance_class                        = var.instance_class
  allocated_storage                     = var.storage_gb
  max_allocated_storage                 = var.max_storage_gb
  storage_type                          = "gp3"
  storage_encrypted                     = true
  kms_key_id                            = var.kms_key_arn
  db_name                               = "microinsurance"
  username                              = "microinsurance_admin"
  manage_master_user_password           = true
  master_user_secret_kms_key_id         = var.kms_key_arn
  multi_az                              = true
  db_subnet_group_name                  = aws_db_subnet_group.this.name
  vpc_security_group_ids                = [var.security_group_id]
  parameter_group_name                  = aws_db_parameter_group.this.name
  publicly_accessible                   = false
  backup_retention_period               = 35
  backup_window                         = var.backup_window
  maintenance_window                    = var.maintenance_window
  copy_tags_to_snapshot                 = true
  deletion_protection                   = true
  skip_final_snapshot                   = false
  final_snapshot_identifier             = "${var.name}-final"
  auto_minor_version_upgrade            = true
  performance_insights_enabled          = true
  performance_insights_kms_key_id       = var.kms_key_arn
  performance_insights_retention_period = 7
  enabled_cloudwatch_logs_exports       = ["postgresql"]
  tags                                  = var.tags
}
