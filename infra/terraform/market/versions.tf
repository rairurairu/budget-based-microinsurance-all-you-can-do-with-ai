terraform {
  required_version = ">= 1.9"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
  # State per market and environment, in the insurer's state bucket (README.md):
  #   terraform init -backend-config=backend-tw-pilot.hcl
  backend "s3" {}
}

provider "aws" {
  region = var.region
  default_tags {
    tags = local.tags
  }
}

# CloudFront's certificate and WAF live in us-east-1.
provider "aws" {
  alias  = "us_east_1"
  region = "us-east-1"
  default_tags {
    tags = local.tags
  }
}
