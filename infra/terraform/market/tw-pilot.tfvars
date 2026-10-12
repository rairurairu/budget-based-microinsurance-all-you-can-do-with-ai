# Taiwan (config/markets/tw, ADR-012), for its pilot: its own region, network, database and keys.
# Illustrative names: the insurer's own.
market                 = "tw"
market_overlay         = "tw"
environment            = "pilot"
region                 = "ap-east-2"
vpc_cidr               = "10.41.0.0/16"
image                  = "<account>.dkr.ecr.ap-east-2.amazonaws.com/microinsurance-tw-pilot:<release>"
origin_domain_name     = "origin.cover.insurer.example.tw"
origin_certificate_arn = "<ACM certificate ARN in ap-east-2>"
public_domain_names    = ["cover.insurer.example.tw"]
edge_certificate_arn   = "<ACM certificate ARN in us-east-1>"
record_retention_years = 7
alarm_emails           = ["<on-call address>"]
