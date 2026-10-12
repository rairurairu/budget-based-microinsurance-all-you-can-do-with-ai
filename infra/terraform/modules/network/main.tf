# One market's network: a VPC across `az_count` availability zones, with public subnets for the
# load balancer and the NAT gateways, private subnets for the API and the workers, and isolated
# subnets for the database (no route out). The trigger feeds and the model provider are reached
# through NAT; S3 through a gateway endpoint, so evidence never leaves the AWS network.

data "aws_availability_zones" "available" {
  state = "available"
}

locals {
  azs = slice(data.aws_availability_zones.available.names, 0, var.az_count)
}

resource "aws_vpc" "this" {
  cidr_block           = var.cidr
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = merge(var.tags, { Name = var.name })
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
  tags   = merge(var.tags, { Name = var.name })
}

resource "aws_subnet" "public" {
  count                   = var.az_count
  vpc_id                  = aws_vpc.this.id
  availability_zone       = local.azs[count.index]
  cidr_block              = cidrsubnet(var.cidr, 8, count.index)
  map_public_ip_on_launch = false
  tags                    = merge(var.tags, { Name = "${var.name}-public-${local.azs[count.index]}" })
}

resource "aws_subnet" "private" {
  count             = var.az_count
  vpc_id            = aws_vpc.this.id
  availability_zone = local.azs[count.index]
  cidr_block        = cidrsubnet(var.cidr, 8, 10 + count.index)
  tags              = merge(var.tags, { Name = "${var.name}-private-${local.azs[count.index]}" })
}

resource "aws_subnet" "database" {
  count             = var.az_count
  vpc_id            = aws_vpc.this.id
  availability_zone = local.azs[count.index]
  cidr_block        = cidrsubnet(var.cidr, 8, 20 + count.index)
  tags              = merge(var.tags, { Name = "${var.name}-database-${local.azs[count.index]}" })
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.this.id
  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this.id
  }
  tags = merge(var.tags, { Name = "${var.name}-public" })
}

resource "aws_route_table_association" "public" {
  count          = var.az_count
  subnet_id      = aws_subnet.public[count.index].id
  route_table_id = aws_route_table.public.id
}

# One NAT gateway per zone in production; one for all zones in a pilot (`nat_per_az = false`).
resource "aws_eip" "nat" {
  count  = var.nat_per_az ? var.az_count : 1
  domain = "vpc"
  tags   = merge(var.tags, { Name = "${var.name}-nat-${count.index}" })
}

resource "aws_nat_gateway" "this" {
  count         = var.nat_per_az ? var.az_count : 1
  allocation_id = aws_eip.nat[count.index].id
  subnet_id     = aws_subnet.public[count.index].id
  tags          = merge(var.tags, { Name = "${var.name}-nat-${count.index}" })
  depends_on    = [aws_internet_gateway.this]
}

resource "aws_route_table" "private" {
  count  = var.az_count
  vpc_id = aws_vpc.this.id
  route {
    cidr_block     = "0.0.0.0/0"
    nat_gateway_id = aws_nat_gateway.this[var.nat_per_az ? count.index : 0].id
  }
  tags = merge(var.tags, { Name = "${var.name}-private-${count.index}" })
}

resource "aws_route_table_association" "private" {
  count          = var.az_count
  subnet_id      = aws_subnet.private[count.index].id
  route_table_id = aws_route_table.private[count.index].id
}

# The database subnets have no route out of the VPC.
resource "aws_route_table" "database" {
  vpc_id = aws_vpc.this.id
  tags   = merge(var.tags, { Name = "${var.name}-database" })
}

resource "aws_route_table_association" "database" {
  count          = var.az_count
  subnet_id      = aws_subnet.database[count.index].id
  route_table_id = aws_route_table.database.id
}

data "aws_region" "current" {}

resource "aws_vpc_endpoint" "s3" {
  vpc_id            = aws_vpc.this.id
  service_name      = "com.amazonaws.${data.aws_region.current.region}.s3"
  vpc_endpoint_type = "Gateway"
  route_table_ids   = aws_route_table.private[*].id
  tags              = merge(var.tags, { Name = "${var.name}-s3" })
}

# Who may reach whom: the internet reaches the load balancer only; the load balancer reaches the
# API; the API and the workers reach the database; nothing else reaches the database.
resource "aws_security_group" "alb" {
  name        = "${var.name}-alb"
  description = "HTTPS from the internet (CloudFront) to the load balancer"
  vpc_id      = aws_vpc.this.id
  tags        = var.tags
}

resource "aws_vpc_security_group_ingress_rule" "alb_https" {
  security_group_id = aws_security_group.alb.id
  description       = "HTTPS"
  ip_protocol       = "tcp"
  from_port         = 443
  to_port           = 443
  prefix_list_id    = var.cloudfront_prefix_list_id
}

resource "aws_vpc_security_group_egress_rule" "alb_to_api" {
  security_group_id            = aws_security_group.alb.id
  description                  = "To the API tasks"
  ip_protocol                  = "tcp"
  from_port                    = var.api_port
  to_port                      = var.api_port
  referenced_security_group_id = aws_security_group.api.id
}

resource "aws_security_group" "api" {
  name        = "${var.name}-api"
  description = "API tasks: from the load balancer"
  vpc_id      = aws_vpc.this.id
  tags        = var.tags
}

resource "aws_vpc_security_group_ingress_rule" "api_from_alb" {
  security_group_id            = aws_security_group.api.id
  description                  = "From the load balancer"
  ip_protocol                  = "tcp"
  from_port                    = var.api_port
  to_port                      = var.api_port
  referenced_security_group_id = aws_security_group.alb.id
}

resource "aws_security_group" "workers" {
  name        = "${var.name}-workers"
  description = "Workers: nothing reaches them"
  vpc_id      = aws_vpc.this.id
  tags        = var.tags
}

# The API and the workers call out over HTTPS (feeds, the model provider, AWS APIs) and reach
# the database.
resource "aws_vpc_security_group_egress_rule" "https_out" {
  for_each          = { api = aws_security_group.api.id, workers = aws_security_group.workers.id }
  security_group_id = each.value
  description       = "HTTPS out"
  ip_protocol       = "tcp"
  from_port         = 443
  to_port           = 443
  cidr_ipv4         = "0.0.0.0/0"
}

resource "aws_vpc_security_group_egress_rule" "to_database" {
  for_each                     = { api = aws_security_group.api.id, workers = aws_security_group.workers.id }
  security_group_id            = each.value
  description                  = "PostgreSQL"
  ip_protocol                  = "tcp"
  from_port                    = 5432
  to_port                      = 5432
  referenced_security_group_id = aws_security_group.database.id
}

resource "aws_security_group" "database" {
  name        = "${var.name}-database"
  description = "PostgreSQL: from the API and the workers only"
  vpc_id      = aws_vpc.this.id
  tags        = var.tags
}

resource "aws_vpc_security_group_ingress_rule" "database_from" {
  for_each                     = { api = aws_security_group.api.id, workers = aws_security_group.workers.id }
  security_group_id            = aws_security_group.database.id
  description                  = "From the ${each.key}"
  ip_protocol                  = "tcp"
  from_port                    = 5432
  to_port                      = 5432
  referenced_security_group_id = each.value
}
