# Terraform (AWS): one deployment per market

ADR-012. Each market runs in its own AWS region, in or near the market, with its own key,
network, database and secrets: Hong Kong in `ap-east-1`, Taiwan in `ap-east-2`. Every market runs
the same image (`infra/Dockerfile`); `market_overlay` picks its config (`config/markets/<market>/`).

```
market/            the root module: one market and environment (hk-pilot.tfvars, tw-pilot.tfvars)
modules/network    VPC; public, private and isolated database subnets; NAT; S3 endpoint;
                   security groups (only CloudFront reaches the load balancer, only the API and
                   the workers reach the database)
modules/database   RDS PostgreSQL 16, Multi-AZ, encrypted, TLS only, 35 days point-in-time
modules/evidence   S3 for evidence: private, KMS, versioned, TLS only
modules/secrets    Secrets Manager containers: api_token_secret, audit_seed, database_url,
                   model_api_key (values set out of band)
modules/service    ECS Fargate: the API behind an ALB, the workers; autoscaling; logs
modules/edge       CloudFront (the apps' static files cached, the API never) and AWS WAF
modules/backup     AWS Backup: daily for 35 days, monthly for record_retention_years, locked vault
modules/monitoring alarms: API 5xx, p99, healthy tasks, database CPU and storage
```

## Deploying a market

1. **State.** An S3 bucket for Terraform state per insurer; a backend file per market and
   environment (`bucket`, `key = "microinsurance/<market>-<environment>.tfstate"`, `region`,
   `use_lockfile = true`), then `terraform -chdir=infra/terraform/market init
   -backend-config=backend-<market>-<environment>.hcl`.
2. **Certificates.** ACM certificates for the public names (in us-east-1, for CloudFront) and for
   the load balancer's own name (in the market's region).
3. **Apply.** `terraform -chdir=infra/terraform/market apply -var-file=tw-pilot.tfvars`.
4. **Secrets.** Store each value with `aws secretsmanager put-secret-value`, never in a variable:
   a random `api_token_secret` of 32 bytes or more; the insurer's `audit_seed` (R14, R59); the
   application role's `database_url` (`postgres://…?sslmode=require`; create the role with the
   master password RDS keeps in Secrets Manager, `database_master_secret_arn`); the model
   provider's key.
5. **Image.** Build for linux/arm64 with the market's filed rating basis and prevention basis
   added at `/app/market-data/` (they are market data, filed by the actuary, not code), push it to
   the market's repository (`image_repository`, immutable tags) and set `image` to it.
6. **Names.** Point the public names at CloudFront (`public_domain_name`) and the origin name at
   the load balancer (`origin_load_balancer`).
7. **Check.** Run the load test against the deployment at the insurer's expected volumes
   (`docs/performance.md`) before the pilot opens.

The workers run two tasks: they share each source's poll (one leader per source) and the job
queue. The API scales between `api_min` and `api_max` on CPU.

## Not here yet

`terraform validate` has not been run against the AWS provider in this repository; DNS records,
the insurer's identity provider and the payment and assistance providers are the insurer's own
(ADR-011).
