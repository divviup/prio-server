variable "peer_share_processor_names" {
  type = list(string)
}

variable "infra_name" {
  type = string
}

variable "container_registry" {
  type = string
}

variable "execution_manager_image" {
  type = string
}

variable "execution_manager_version" {
  type = string
}

resource "kubernetes_namespace" "namespaces" {
  count = length(var.peer_share_processor_names)

  metadata {
    name = var.peer_share_processor_names[count.index]
    annotations = {
      infra = var.infra_name
    }
  }
}

resource "kubernetes_cron_job" "execution_managers" {
  count = length(var.peer_share_processor_names)
  metadata {
    name      = "${var.infra_name}-execution-manager"
    namespace = var.peer_share_processor_names[count.index]

    annotations = {
      infra = var.infra_name
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
