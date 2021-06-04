variable "environment" {
  type = string
}

variable "data_share_processor_name" {
  type = string
}

variable "publisher_iam_role" {
  type = string
}

variable "subscriber_iam_role" {
  type = string
}

variable "task" {
  type = string
}

resource "aws_sns_topic" "task" {
  name = "${var.environment}-${var.data_share_processor_name}-${var.task}"
}

resource "aws_sns_topic_policy" "task" {
  arn = aws_sns_topic.task.arn

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          AWS = var.publisher_iam_role
        }
        Action = [
          "SNS:Publish",
        ]
        Resource = aws_sns_topic.task.arn
      }
    ]
  })
}

resource "aws_sqs_queue" "task" {
  name                       = "${var.environment}-${var.data_share_processor_name}-${var.task}"
  visibility_timeout_seconds = 600
  redrive_policy = jsonencode({
    deadLetterTargetArn = aws_sqs_queue.dead_letter.arn
    maxReceiveCount     = 5
  })
}

resource "aws_sqs_queue_policy" "task" {
  queue_url = aws_sqs_queue.task.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Service = "sns.amazonaws.com"
        }
        Action = [
          "sqs:SendMessage"
        ]
        Resource = aws_sqs_queue.task.arn
        Condition = {
          ArnEquals = {
            "aws:SourceArn" = aws_sns_topic.task.arn
          }
        }
      },
      {
        Effect = "Allow"
        Principal = {
          AWS = var.subscriber_iam_role
        }
        Action = [
          "sqs:ChangeMessageVisibility",
          "sqs:DeleteMessage",
          "sqs:ReceiveMessage",
        ]
        Resource = aws_sqs_queue.task.arn
      }
    ]
  })
}

resource "aws_sns_topic_subscription" "task" {
  topic_arn            = aws_sns_topic.task.arn
  protocol             = "sqs"
  endpoint             = aws_sqs_queue.task.arn
  raw_message_delivery = true
}

resource "aws_sqs_queue" "dead_letter" {
  name                       = "${aws_sns_topic.task.name}-dead-letter"
  visibility_timeout_seconds = 600
  message_retention_seconds  = 1209600
}

resource "aws_sqs_queue_policy" "dead_letter" {
  queue_url = aws_sqs_queue.dead_letter.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Principal = {
          Service = "sqs.amazonaws.com"
        }
        Action = [
          "sqs:SendMessage"
        ]
        Resource = aws_sqs_queue.dead_letter.arn
      }
    ]
  })
}

output "topic" {
  value = aws_sns_topic.task.arn
}

# aws_sqs_queue.id yields the SQS queue *URL*, not an ARN or name, which is
# what clients need in order to dequeue messages
output "subscription" {
  value = aws_sqs_queue.task.id
}
