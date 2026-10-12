# The market's one public origin: CloudFront in front of the load balancer, with AWS WAF. The
# apps are served by the API service on the same origin (ADR-012), so they call the API on their
# own origin and a customer's token never crosses one; partners' pages load the embed from here
# and call the API under CORS for their registered origins (ADR-011). Only the apps' static files
# are cached; every API response passes through uncached, so no customer's data rests at the edge.

terraform {
  required_providers {
    aws = {
      source                = "hashicorp/aws"
      configuration_aliases = [aws.us_east_1]
    }
  }
}

data "aws_cloudfront_cache_policy" "disabled" {
  name = "Managed-CachingDisabled"
}

data "aws_cloudfront_cache_policy" "optimized" {
  name = "Managed-CachingOptimized"
}

data "aws_cloudfront_origin_request_policy" "all_viewer" {
  name = "Managed-AllViewer"
}

# CloudFront's WAF lives in us-east-1. The common rule set, known bad inputs, and a rate limit per
# client address.
resource "aws_wafv2_web_acl" "this" {
  provider = aws.us_east_1
  name     = var.name
  scope    = "CLOUDFRONT"
  default_action {
    allow {}
  }
  dynamic "rule" {
    for_each = {
      AWSManagedRulesCommonRuleSet          = 10
      AWSManagedRulesKnownBadInputsRuleSet  = 20
      AWSManagedRulesAmazonIpReputationList = 30
    }
    content {
      name     = rule.key
      priority = rule.value
      override_action {
        none {}
      }
      statement {
        managed_rule_group_statement {
          vendor_name = "AWS"
          name        = rule.key
        }
      }
      visibility_config {
        cloudwatch_metrics_enabled = true
        metric_name                = rule.key
        sampled_requests_enabled   = true
      }
    }
  }
  rule {
    name     = "rate-per-address"
    priority = 40
    action {
      block {}
    }
    statement {
      rate_based_statement {
        limit              = var.requests_per_5_minutes
        aggregate_key_type = "IP"
      }
    }
    visibility_config {
      cloudwatch_metrics_enabled = true
      metric_name                = "rate-per-address"
      sampled_requests_enabled   = true
    }
  }
  visibility_config {
    cloudwatch_metrics_enabled = true
    metric_name                = var.name
    sampled_requests_enabled   = true
  }
  tags = var.tags
}

resource "aws_cloudfront_distribution" "this" {
  enabled         = true
  comment         = var.name
  aliases         = var.aliases
  price_class     = var.price_class
  http_version    = "http2and3"
  is_ipv6_enabled = true
  web_acl_id      = aws_wafv2_web_acl.this.arn

  origin {
    origin_id   = "api"
    domain_name = var.origin_domain_name
    custom_origin_config {
      http_port              = 80
      https_port             = 443
      origin_protocol_policy = "https-only"
      origin_ssl_protocols   = ["TLSv1.2"]
      origin_read_timeout    = 60
    }
  }

  # The API: never cached; every header, including the bearer token, passes through.
  default_cache_behavior {
    target_origin_id         = "api"
    viewer_protocol_policy   = "redirect-to-https"
    allowed_methods          = ["GET", "HEAD", "OPTIONS", "PUT", "POST", "PATCH", "DELETE"]
    cached_methods           = ["GET", "HEAD"]
    cache_policy_id          = data.aws_cloudfront_cache_policy.disabled.id
    origin_request_policy_id = data.aws_cloudfront_origin_request_policy.all_viewer.id
    compress                 = true
  }

  # The apps' static files. The ops console shares its prefix with API routes (/ops/queue), so it
  # is not cached here.
  dynamic "ordered_cache_behavior" {
    for_each = toset(["/app/*", "/embed/*", "/partner/*"])
    content {
      path_pattern           = ordered_cache_behavior.value
      target_origin_id       = "api"
      viewer_protocol_policy = "redirect-to-https"
      allowed_methods        = ["GET", "HEAD"]
      cached_methods         = ["GET", "HEAD"]
      cache_policy_id        = data.aws_cloudfront_cache_policy.optimized.id
      compress               = true
    }
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = var.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }

  tags = var.tags
}
