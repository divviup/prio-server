.ONESHELL:
.SHELL := /usr/bin/bash
BOLD=$(shell tput bold)
RED=$(shell tput setaf 1)
GREEN=$(shell tput setaf 2)
YELLOW=$(shell tput setaf 3)
RESET=$(shell tput sgr0)

ifeq ($(ENV),)
$(info $(BOLD)Example usage: \`ENV=demo make plan\`$(RESET))
$(error $(BOLD)$(RED)ENV was not set$(RESET))
endif

VARS=variables/$(ENV).tfvars
REGION="$(shell grep '^gcp_region' "$(VARS)" | awk -F'=' '{print $$2}' | sed 's/[[:space:]]//g')"
ifeq ($(REGION),"")
$(error $(BOLD)$(RED)REGION was not detected$(RESET))
endif
PROJECT="$(shell grep '^gcp_project' "$(VARS)" | awk -F'=' '{print $$2}' | sed 's/[[:space:]]//g')"
ifeq ($(PROJECT),"")
$(error $(BOLD)$(RED)PROJECT was not detected$(RESET))
endif
STATE_BUCKET="$(shell grep '^state_bucket' "$(VARS)" | awk -F'=' '{print $$2}' | sed 's/[[:space:]]//g')"
ifeq ($(STATE_BUCKET),"")
$(error $(BOLD)$(RED)STATE_BUCKET was not detected$(RESET))
endif

.PHONY: help
help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: prep
prep: ## Prepare a new workspace (environment) if needed, configure the tfstate backend, update any modules, and switch to the workspace
	@rm -f .terraform/terraform.tfstate
	@if [ ! -f "$(VARS)" ]; then \
		echo "$(BOLD)$(RED)Could not find variables file: $(VARS)$(RESET)"; \
		exit 1; \
	 fi
	@echo "$(BOLD)Configuring the terraform backend$(RESET)"
	@terraform init \
		-input=false \
		-force-copy \
		-upgrade \
		-backend=true \
		-backend-config="bucket=$(STATE_BUCKET)"

.PHONY: validate
validate: prep ## Show what terraform believes about the validity of the defined resources
	@terraform validate

.PHONY: format
format: prep ## Rewrites all Terraform configuration files to a canonical format.
	@terraform fmt -recursive

.PHONY: providers
providers: prep ## Review provider information in terraform state
	@terraform providers

.PHONY: plan
plan: prep ## Show what terraform thinks it will do
	@terraform plan \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)"

.PHONY: plan-target
plan-target: prep ## Shows what a plan looks like for applying a specific resource
	@echo "$(YELLOW)$(BOLD)[INFO]   $(RESET)"; echo "Example to type for the following question: module.gke.google_container_cluster.cluster"
	@read -p "PLAN target: " DATA && \
		terraform plan \
			-lock=true \
			-input=true \
			-refresh=true \
			-var-file="$(VARS)" \
			-target=$$DATA

.PHONY: plan-destroy
plan-destroy: prep ## Creates a destruction plan.
	@terraform plan \
		-input=false \
		-refresh=true \
		-destroy \
		-var-file="$(VARS)"

.PHONY: output
output: prep ## Make Terraform print output variable(s).
	@terraform output

.PHONY: apply
apply: prep ## Have terraform do the things. This will cost money.
	@terraform apply \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)"

.PHONY: plan-bootstrap
plan-bootstrap: prep ## Show what terraform thinks it will do
	@terraform plan \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)" \
		-target=module.manifest_gcp \
		-target=module.manifest_aws \
		-target=module.fake_server_resources[0].google_storage_bucket_object.global_manifest \
		-target=module.portal_server_resources \
		-target=module.gke \
		-target=module.eks \
		-target=module.custom_metrics

.PHONY: apply-bootstrap
apply-bootstrap: prep ## Have terraform bring up the minimal resources needed to bootstrap an env and permit peers to begin deploying
	@terraform apply \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)" \
		-target=module.manifest_gcp \
		-target=module.manifest_aws \
		-target=module.fake_server_resources[0].google_storage_bucket_object.global_manifest \
		-target=module.portal_server_resources \
		-target=module.gke \
		-target=module.eks \
		-target=module.custom_metrics

.PHONY: destroy-bootstrap
destroy-bootstrap: prep ## Have terraform destroy the resources brought up by apply-bootstrap
	@terraform destroy \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)" \
		-target=module.manifest_gcp \
		-target=module.manifest_aws \
		-target=module.fake_server_resources[0].google_storage_bucket_object.global_manifest \
		-target=module.portal_server_resources \
		-target=module.gke \
		-target=module.eks \
		-target=module.custom_metrics

.PHONY: apply-target
apply-target: prep ## Have terraform do the things for a specific resource. This will cost money.
	@echo "$(YELLOW)$(BOLD)[INFO]   $(RESET)"; echo "Example to type for the following question: module.gke.google_container_cluster.cluster"
	@read -p "APPLY target: " DATA && \
		terraform apply \
			-lock=true \
			-input=true \
			-refresh=true \
			-var-file="$(VARS)" \
			-target=$$DATA

.PHONY: destroy
destroy: prep ## Destroy the things
	@terraform destroy \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="$(VARS)"

.PHONY: destroy-target
destroy-target: prep ## Destroy a specific resource. Caution though, this destroys chained resources.
	@echo "$(YELLOW)$(BOLD)[INFO] Specifically destroy a piece of Terraform data.$(RESET)"; echo "Example to type for the following question: module.gke.google_container_cluster.cluster"
	@read -p "Destroy target: " DATA && \
		terraform destroy \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file=$(VARS) \
		-target=$$DATA

.PHONY: prep-cluster-bootstrap
prep-cluster-bootstrap: ## Prepare a new workspace (environment) if needed, configure the tfstate backend, update any modules, and switch to the workspace
	@rm -f cluster_bootstrap/.terraform/terraform.tfstate
	@if [ ! -f "$(VARS)" ]; then \
		echo "$(BOLD)$(RED)Could not find variables file: $(VARS)$(RESET)"; \
		exit 1; \
	 fi
	@echo "$(BOLD)Configuring the terraform backend$(RESET)"
	@terraform \
		-chdir="cluster_bootstrap" \
		init \
		-input=false \
		-force-copy \
		-upgrade \
		-backend=true \
		-backend-config="bucket=$(STATE_BUCKET)" \
		-backend-config="prefix=cluster-bootstrap"

.PHONY: plan-cluster-bootstrap
plan-cluster-bootstrap: prep-cluster-bootstrap ## Show plan for configuring AWS management account for this environment
	@terraform \
		-chdir="cluster_bootstrap" \
		plan \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="../$(VARS)"

.PHONY: apply-cluster-bootstrap
apply-cluster-bootstrap: prep-cluster-bootstrap
	@terraform \
		-chdir="cluster_bootstrap" \
		apply \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="../$(VARS)"

.PHONY: plan-destroy-cluster-bootstrap
plan-destroy-cluster-bootstrap: prep-cluster-bootstrap
	@terraform \
		-chdir="cluster_bootstrap" \
		plan \
		-lock=true \
		-input=false \
		-refresh=true \
		-destroy \
		-var-file="../$(VARS)"

.PHONY: destroy-cluster-bootstrap
destroy-cluster-bootstrap: prep-cluster-bootstrap
	@terraform \
		-chdir="cluster_bootstrap" \
		destroy \
		-lock=true \
		-input=false \
		-refresh=true \
		-var-file="../$(VARS)"
