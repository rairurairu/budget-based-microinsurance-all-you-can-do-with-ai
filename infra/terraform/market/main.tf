# One market's deployment (ADR-012): a region in or near the market for data residency, its own
# network, database, evidence store, secrets, processes, edge, backups and alarms. Every market
# runs the same image; `market_overlay` picks its config (config/markets/<market>/).

locals {
  name = "microinsurance-${var.market}-${var.environment}"
  tags = merge(var.tags, {
    Product     = "budget-based-microinsurance"
    Market      = var.market
    Environment = var.environment
  })
}

data "aws_caller_identity" "current" {}

data "aws_region" "current" {}

data "aws_ec2_managed_prefix_list" "cloudfront" {
  name = "com.amazonaws.global.cloudfront.origin-facing"
}

# The market's key: the database, evidence, secrets, logs, backups and alarms. Rotated yearly.
data "aws_iam_policy_document" "key" {
  statement {
    sid       = "Account"
    actions   = ["kms:*"]
    resources = ["*"]
    principals {
      type        = "AWS"
      identifiers = ["arn:aws:iam::${data.aws_caller_identity.current.account_id}:root"]
    }
  }
  statement {
    sid = "Logs"
    actions = [
      "kms:Encrypt", "kms:Decrypt", "kms:ReEncrypt*", "kms:GenerateDataKey*", "kms:DescribeKey",
    ]
    resources = ["*"]
    principals {
      type        = "Service"
      identifiers = ["logs.${data.aws_region.current.region}.amazonaws.com"]
    }
  }
  statement {
    sid       = "Alarms"
    actions   = ["kms:Decrypt", "kms:GenerateDataKey*"]
    resources = ["*"]
    principals {
      type        = "Service"
      identifiers = ["cloudwatch.amazonaws.com"]
    }
  }
}

resource "aws_kms_key" "market" {
  description             = "${local.name}: records, evidence, secrets, logs and backups"
  enable_key_rotation     = true
  deletion_window_in_days = 30
  policy                  = data.aws_iam_policy_document.key.json
}

resource "aws_kms_alias" "market" {
  name          = "alias/${local.name}"
  target_key_id = aws_kms_key.market.key_id
}

# The image registry: immutable tags, scanned on push.
resource "aws_ecr_repository" "this" {
  name                 = local.name
  image_tag_mutability = "IMMUTABLE"
  image_scanning_configuration {
    scan_on_push = true
  }
  encryption_configuration {
    encryption_type = "KMS"
    kms_key         = aws_kms_key.market.arn
  }
}

module "network" {
  source                    = "../modules/network"
  name                      = local.name
  cidr                      = var.vpc_cidr
  az_count                  = var.az_count
  nat_per_az                = var.environment == "production"
  cloudfront_prefix_list_id = data.aws_ec2_managed_prefix_list.cloudfront.id
}

module "database" {
  source            = "../modules/database"
  name              = local.name
  subnet_ids        = module.network.database_subnet_ids
  security_group_id = module.network.security_group_ids["database"]
  kms_key_arn       = aws_kms_key.market.arn
  instance_class    = var.db_instance_class
}

module "evidence" {
  source      = "../modules/evidence"
  name        = "${local.name}-evidence-${data.aws_caller_identity.current.account_id}"
  kms_key_arn = aws_kms_key.market.arn
}

module "secrets" {
  source      = "../modules/secrets"
  name        = local.name
  kms_key_arn = aws_kms_key.market.arn
}

module "service" {
  source              = "../modules/service"
  name                = local.name
  image               = var.image
  market_overlay      = var.market_overlay
  evidence_bucket     = module.evidence.bucket
  evidence_bucket_arn = module.evidence.arn
  secret_arns         = module.secrets.arns
  kms_key_arn         = aws_kms_key.market.arn
  vpc_id              = module.network.vpc_id
  public_subnet_ids   = module.network.public_subnet_ids
  private_subnet_ids  = module.network.private_subnet_ids
  security_group_ids  = module.network.security_group_ids
  certificate_arn     = var.origin_certificate_arn
  api_min             = var.api_min
  api_max             = var.api_max
  workers             = var.workers
}

module "edge" {
  source             = "../modules/edge"
  providers          = { aws = aws, aws.us_east_1 = aws.us_east_1 }
  name               = local.name
  origin_domain_name = var.origin_domain_name
  aliases            = var.public_domain_names
  certificate_arn    = var.edge_certificate_arn
}

module "backup" {
  source                 = "../modules/backup"
  name                   = local.name
  kms_key_arn            = aws_kms_key.market.arn
  resource_arns          = [module.database.arn]
  record_retention_years = var.record_retention_years
}

module "monitoring" {
  source                  = "../modules/monitoring"
  name                    = local.name
  kms_key_arn             = aws_kms_key.market.arn
  alarm_emails            = var.alarm_emails
  alb_arn_suffix          = module.service.alb_arn_suffix
  target_group_arn_suffix = module.service.target_group_arn_suffix
  db_identifier           = module.database.identifier
}
