# Hong Kong, the base market, for the pilot. Illustrative names: the insurer's own.
market                 = "hk"
market_overlay         = ""
environment            = "pilot"
region                 = "ap-east-1"
vpc_cidr               = "10.40.0.0/16"
image                  = "<account>.dkr.ecr.ap-east-1.amazonaws.com/microinsurance-hk-pilot:<release>"
origin_domain_name     = "origin.cover.insurer.example"
origin_certificate_arn = "<ACM certificate ARN in ap-east-1>"
public_domain_names    = ["cover.insurer.example"]
edge_certificate_arn   = "<ACM certificate ARN in us-east-1>"
record_retention_years = 7
alarm_emails           = ["<on-call address>"]
