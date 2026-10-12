# The market's processes on ECS Fargate, from one image (infra/Dockerfile): the API behind an
# application load balancer, and the workers, which share the trigger polls (one leader per
# source, an advisory lock) and the job queue (FOR UPDATE SKIP LOCKED) however many run. Both read
# the market from `MARKET` and their secrets from Secrets Manager, never from variables.

data "aws_region" "current" {}

resource "aws_ecs_cluster" "this" {
  name = var.name
  setting {
    name  = "containerInsights"
    value = "enabled"
  }
  tags = var.tags
}

resource "aws_cloudwatch_log_group" "this" {
  for_each          = toset(["api", "workers"])
  name              = "/ecs/${var.name}/${each.key}"
  retention_in_days = var.log_retention_days
  kms_key_id        = var.kms_key_arn
  tags              = var.tags
}

data "aws_iam_policy_document" "assume_ecs_tasks" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
  }
}

# Starts the tasks: pulls the image, reads the secrets, writes the logs.
resource "aws_iam_role" "execution" {
  name               = "${var.name}-execution"
  assume_role_policy = data.aws_iam_policy_document.assume_ecs_tasks.json
  tags               = var.tags
}

resource "aws_iam_role_policy_attachment" "execution" {
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

data "aws_iam_policy_document" "secrets" {
  statement {
    actions   = ["secretsmanager:GetSecretValue"]
    resources = values(var.secret_arns)
  }
  statement {
    actions   = ["kms:Decrypt"]
    resources = [var.kms_key_arn]
  }
}

resource "aws_iam_role_policy" "execution_secrets" {
  name   = "secrets"
  role   = aws_iam_role.execution.id
  policy = data.aws_iam_policy_document.secrets.json
}

# What the running processes may do: read and write evidence, nothing else in AWS.
resource "aws_iam_role" "task" {
  name               = "${var.name}-task"
  assume_role_policy = data.aws_iam_policy_document.assume_ecs_tasks.json
  tags               = var.tags
}

data "aws_iam_policy_document" "evidence" {
  statement {
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${var.evidence_bucket_arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [var.evidence_bucket_arn]
  }
  statement {
    actions   = ["kms:Decrypt", "kms:GenerateDataKey"]
    resources = [var.kms_key_arn]
  }
}

resource "aws_iam_role_policy" "task_evidence" {
  name   = "evidence"
  role   = aws_iam_role.task.id
  policy = data.aws_iam_policy_document.evidence.json
}

locals {
  environment = [
    for k, v in merge(
      {
        MARKET           = var.market_overlay
        PORT             = tostring(var.api_port)
        RATING_BASIS     = var.rating_basis_path
        PREVENTION_BASIS = var.prevention_basis_path
        EVIDENCE_BUCKET  = var.evidence_bucket
      },
      var.insurer_profile == "" ? {} : { INSURER_PROFILE = var.insurer_profile },
    ) : { name = k, value = v }
  ]
  secrets = [
    { name = "API_TOKEN_SECRET", valueFrom = var.secret_arns["api_token_secret"] },
    { name = "AUDIT_SEED", valueFrom = var.secret_arns["audit_seed"] },
    { name = "DATABASE_URL", valueFrom = var.secret_arns["database_url"] },
    { name = "ANTHROPIC_API_KEY", valueFrom = var.secret_arns["model_api_key"] },
  ]
  container = {
    api = {
      command = ["microinsurance-api"]
      ports   = [{ containerPort = var.api_port, protocol = "tcp" }]
    }
    workers = {
      command = ["microinsurance-workers"]
      ports   = []
    }
  }
}

resource "aws_ecs_task_definition" "this" {
  for_each                 = local.container
  family                   = "${var.name}-${each.key}"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.task_cpu[each.key]
  memory                   = var.task_memory[each.key]
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.task.arn
  runtime_platform {
    operating_system_family = "LINUX"
    cpu_architecture        = "ARM64"
  }
  container_definitions = jsonencode([
    {
      name                   = each.key
      image                  = var.image
      command                = each.value.command
      essential              = true
      readonlyRootFilesystem = true
      portMappings           = each.value.ports
      environment            = local.environment
      secrets                = local.secrets
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          awslogs-group         = aws_cloudwatch_log_group.this[each.key].name
          awslogs-region        = data.aws_region.current.region
          awslogs-stream-prefix = each.key
        }
      }
    }
  ])
  tags = var.tags
}

resource "aws_lb" "this" {
  name                       = var.name
  load_balancer_type         = "application"
  internal                   = false
  subnets                    = var.public_subnet_ids
  security_groups            = [var.security_group_ids["alb"]]
  drop_invalid_header_fields = true
  idle_timeout               = 300 # the customer's payout stream stays open (server-sent events)
  enable_deletion_protection = true
  tags                       = var.tags
}

resource "aws_lb_target_group" "api" {
  name                 = "${var.name}-api"
  port                 = var.api_port
  protocol             = "HTTP"
  target_type          = "ip"
  vpc_id               = var.vpc_id
  deregistration_delay = 30
  health_check {
    path                = "/health"
    matcher             = "200"
    healthy_threshold   = 2
    unhealthy_threshold = 3
    interval            = 15
  }
  tags = var.tags
}

resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.this.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = var.certificate_arn
  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.api.arn
  }
  tags = var.tags
}

resource "aws_ecs_service" "api" {
  name            = "api"
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.this["api"].arn
  desired_count   = var.api_min
  launch_type     = "FARGATE"
  network_configuration {
    subnets          = var.private_subnet_ids
    security_groups  = [var.security_group_ids["api"]]
    assign_public_ip = false
  }
  load_balancer {
    target_group_arn = aws_lb_target_group.api.arn
    container_name   = "api"
    container_port   = var.api_port
  }
  deployment_circuit_breaker {
    enable   = true
    rollback = true
  }
  propagate_tags = "SERVICE"
  lifecycle {
    ignore_changes = [desired_count]
  }
  depends_on = [aws_lb_listener.https]
  tags       = var.tags
}

resource "aws_ecs_service" "workers" {
  name            = "workers"
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.this["workers"].arn
  desired_count   = var.workers
  launch_type     = "FARGATE"
  network_configuration {
    subnets          = var.private_subnet_ids
    security_groups  = [var.security_group_ids["workers"]]
    assign_public_ip = false
  }
  deployment_circuit_breaker {
    enable   = true
    rollback = true
  }
  propagate_tags = "SERVICE"
  tags           = var.tags
}

# The API scales on CPU between `api_min` and `api_max` tasks: a typhoon day's payout views and
# claims arrive together.
resource "aws_appautoscaling_target" "api" {
  service_namespace  = "ecs"
  resource_id        = "service/${aws_ecs_cluster.this.name}/${aws_ecs_service.api.name}"
  scalable_dimension = "ecs:service:DesiredCount"
  min_capacity       = var.api_min
  max_capacity       = var.api_max
}

resource "aws_appautoscaling_policy" "api_cpu" {
  name               = "${var.name}-api-cpu"
  policy_type        = "TargetTrackingScaling"
  service_namespace  = aws_appautoscaling_target.api.service_namespace
  resource_id        = aws_appautoscaling_target.api.resource_id
  scalable_dimension = aws_appautoscaling_target.api.scalable_dimension
  target_tracking_scaling_policy_configuration {
    target_value = 60
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageCPUUtilization"
    }
    scale_in_cooldown  = 300
    scale_out_cooldown = 60
  }
}
