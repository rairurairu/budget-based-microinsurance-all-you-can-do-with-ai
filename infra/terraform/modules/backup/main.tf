# Backups kept as long as the records they hold (R54): the database's own point-in-time recovery
# covers 35 days; AWS Backup keeps a monthly copy for `record_retention_years`, in a vault locked
# against deletion (write once, read many), encrypted with the market's key.

resource "aws_backup_vault" "this" {
  name        = var.name
  kms_key_arn = var.kms_key_arn
  tags        = var.tags
}

resource "aws_backup_vault_lock_configuration" "this" {
  backup_vault_name   = aws_backup_vault.this.name
  min_retention_days  = 35
  max_retention_days  = var.record_retention_years * 366
  changeable_for_days = var.lock_changeable_for_days
}

resource "aws_backup_plan" "this" {
  name = var.name
  rule {
    rule_name         = "daily"
    target_vault_name = aws_backup_vault.this.name
    schedule          = "cron(0 19 * * ? *)" # 03:00 at UTC+8
    lifecycle {
      delete_after = 35
    }
  }
  rule {
    rule_name         = "monthly-records"
    target_vault_name = aws_backup_vault.this.name
    schedule          = "cron(0 20 1 * ? *)"
    lifecycle {
      delete_after = var.record_retention_years * 366
    }
  }
  tags = var.tags
}

data "aws_iam_policy_document" "assume_backup" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["backup.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "backup" {
  name               = "${var.name}-backup"
  assume_role_policy = data.aws_iam_policy_document.assume_backup.json
  tags               = var.tags
}

resource "aws_iam_role_policy_attachment" "backup" {
  role       = aws_iam_role.backup.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSBackupServiceRolePolicyForBackup"
}

resource "aws_backup_selection" "this" {
  name         = var.name
  plan_id      = aws_backup_plan.this.id
  iam_role_arn = aws_iam_role.backup.arn
  resources    = var.resource_arns
}
