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

VARS="variables/$(ENV).tfvars"
REGION="$(shell grep '^gcp_region' $(VARS) | awk -F'=' '{print $$2}' | sed 's/[[:space:]]//g')"
ifeq ($(REGION),)
$(error $(BOLD)$(RED)REGION was not detected$(RESET))
endif

STORAGE_BUCKET="$(ENV)-$(REGION)-prio-terraform"
STORAGE_BUCKET_URL="gs://$(STORAGE_BUCKET)"

.PHONY: help
help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: prep
prep: ## Prepare a new workspace (environment) if needed, configure the tfstate backend, update any modules, and switch to the workspace
	@rm -rf .terraform/
	@if [ ! -f "$(VARS)" ]; then \
		echo "$(BOLD)$(RED)Could not find variables file: $(VARS)$(RESET)"; \
		exit 1; \
	 fi
	@echo "$(BOLD)Verifying that the storage bucket $(STORAGE_BUCKET_URL) for remote state exists$(RESET)"
	@if ! gsutil ls -b $(STORAGE_BUCKET_URL) > /dev/null 2>&1 ; then \
		echo "$(BOLD)Storage bucket $(STORAGE_BUCKET_URL) was not found, creating new bucket with versioning enabled to store tfstate$(RESET)"; \
		gsutil mb -l $(REGION) $(STORAGE_BUCKET_URL) > /dev/null 2>&1 ; \
		gsutil versioning set on $(STORAGE_BUCKET_URL) > /dev/null 2>&1 ; \
		echo "$(BOLD)$(GREEN)Storage bucket $(STORAGE_BUCKET_URL) created$(RESET)"; \
	 else
		echo "$(BOLD)$(GREEN)Storage bucket $(STORAGE_BUCKET_URL) exists$(RESET)"; \
	 fi
	@echo "$(BOLD)Configuring the terraform backend$(RESET)"
	@terraform init \
		-input=false \
		-force-copy \
		-lock=true \
		-upgrade \
		-verify-plugins=true \
		-backend=true \
		-backend-config="bucket=$(STORAGE_BUCKET)"

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

.PHONY: destroy-backend
destroy-backend: ## Destroy storage bucket
	@if ! gsutil rm -r $(STORAGE_BUCKET_URL); \
		echo "$(BOLD)$(RED)Unable to delete cloud storage bucket $(STORAGE_BUCKET_URL).$(RESET)"; \
	else
		echo "$(BOLD)$(RED)Cloud storage bucket $(STORAGE_BUCKET_URL) does not exist.$(RESET)"; \
	fi
