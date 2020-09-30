variable "peer_share_processor_name" {
  type = string
}

variable "environment" {
  type = string
}

variable "container_registry" {
  type    = string
  default = "letsencrypt"
}

variable "execution_manager_image" {
  type    = string
  default = "prio-facilitator"
}

variable "execution_manager_version" {
  type    = string
  default = "0.1.0"
}

resource "kubernetes_namespace" "namespace" {
  metadata {
    name = var.peer_share_processor_name
    annotations = {
      environment = var.environment
    }
  }
}

resource "kubernetes_cron_job" "execution_manager" {
  metadata {
    name      = "${var.environment}-execution-manager"
    namespace = var.peer_share_processor_name

    annotations = {
      environment = var.environment
    }
  }
  spec {
    schedule = "*/10 * * * *"
    job_template {
      metadata {}
      spec {
        template {
          metadata {}
          spec {
            # Credentials are needed to pull images from Google Container Registry
            automount_service_account_token = true
            container {
              # For now just have the facilitator print its help text
              name  = "facilitator"
              image = "${var.container_registry}/${var.execution_manager_image}:${var.execution_manager_version}"
              args  = ["--help"]
            }
            restart_policy = "OnFailure"
          }
        }
      }
    }
  }
}
