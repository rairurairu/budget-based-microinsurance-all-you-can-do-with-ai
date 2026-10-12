# The secrets a market's processes need, as empty containers: the operator stores each value out
# of band (README.md), so no secret is ever in Terraform state or variables.
#
#   api_token_secret   signs every principal's token, 32 bytes at least (ADR-011)
#   audit_seed         the insurer's secret audit seed, recorded with each sample (R14, R59)
#   database_url       the application role's connection URL, TLS required
#   model_api_key      the AI layer's model provider key; without it every AI component gives its
#                      saved answer (R22)

locals {
  names = ["api_token_secret", "audit_seed", "database_url", "model_api_key"]
}

resource "aws_secretsmanager_secret" "this" {
  for_each                = toset(local.names)
  name                    = "${var.name}/${each.key}"
  kms_key_id              = var.kms_key_arn
  recovery_window_in_days = 30
  tags                    = var.tags
}
