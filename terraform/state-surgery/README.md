# state-surgery

Deploying `prio-server` in support of international ENPA required some
[significant refactoring of Terraform modules](https://github.com/abetterinternet/prio-server/issues/655).
That meant that the existing `prod-us` environment's `.tfstate` lost track of
managed resources, which had moved to new paths in the refactored modules. This
is a guide to performing surgery on the Terraform state of an existing
environment to bring it under the management of the refactored Terraform
modules.

Note that the guide here and the `./state-surgery.sh` script apply only to the
Terraform module refactor done in July/August 2021.

## Migrating an environment

First, we must get a copy remote state so we can safely operate on it without
damaging the remote state. Once you have initialized Terraform, you can use TF
to fetch a copy of state:

    ENV=<env> TF_VAR_aws_profile=<profile> make prep
    terraform state pull > <local path>

We are going to operate on that local copy, and we make a further backup of it
so that we can always restore Terraform to a known state.

Next, we must make sure we can perform Terraform actions against that local
state to check that the state modifications we will later make worked. Apply
changes like this diff to `terraform/main.tf` (this exact patch may not apply
cleanly by the time you read this) so that `terraform plan` will not use the
remote state bucket:

```
diff --git a/terraform/main.tf b/terraform/main.tf
index a409873..cab1db1 100644
--- a/terraform/main.tf
+++ b/terraform/main.tf
@@ -199,7 +199,9 @@ variable "cluster_settings" {
 }

 terraform {
-  backend "gcs" {}
+  backend "local" {
+    path = "<local path>"
+  }

   required_version = ">= 0.14.4"

@@ -246,16 +248,6 @@ terraform {
   }
 }

-data "terraform_remote_state" "state" {
-  backend = "gcs"
-
-  workspace = "${var.environment}-${var.gcp_region}"
-
-  config = {
-    bucket = "${var.environment}-${var.gcp_region}-prio-terraform"
-  }
-}
-
 data "google_project" "current" {}
 data "google_client_config" "current" {}
 data "aws_caller_identity" "current" {}
```

Then `init` and `plan`, to prove we can work against our local tfstate file.

    rm -rf /path/to/prio-server/terraform/.terraform
    terraform init --lock=false --upgrade --verify-plugins=true --backend=true
    terraform plan --state=<local path> --input=false --var-file=variables/<env>.tfvars --refresh=true --lock=false

The plan output should be empty or correspond to the stuff we haven't moved
yet. If it wants to rebuild the whole world from scratch, we did it wrong. If
the plan output looks reasonable, then we are ready to move stuff:

    ./state-surgery.sh <local path> "namespace-1 namespace-2 ..." "ingestor-1 ingestor-2 ..."

The arguments to `state-surgery.sh` are positional:
 - `$1` is the path to the local `.tfstate` file
 - `$2` is a list of namespaces (a.k.a. localities) in the environment
 - `$3` is a list of ingestors, which are assumed to exist in each locality
 - `$4` (optional) is a list of extra arguments transparently passed through
   to `terraform state mv`. For instance, set `--dry-run` to see what _would_
   get moved.

After doing surgery, we run `terraform plan` again (with args as above) and we
should see fewer items in the plan if we moved resources into the correct place.

When we are satisfied with the changes, we push the modified state back into the
remote backend place. First, we remove the change made to `main.tf`. Then we
re-init terraform, since the `terraform.backend` block in `main.tf` has changed:

    ENV=<env> TF_VAR_aws_profile=<profile> make prep

Then we push the state to the backend (i.e., upload to the GCS bucket):

    terraform state push <local path>

Finally, check `terraform plan` output to make sure it is as expected.

    ENV=<env> TF_VAR_aws_profile=<profile> make plan

If terraform emits a message about minor deviations in non-critical pieces of
the state, like:

   `~ resource_version = "156735235" -> "156738850"`

and suggests providing the `-refresh-only` flag to `plan` and `apply`, then
utilize the `plan-refresh` and `apply-refresh` `make` tarets to update the
remote state.

    ENV=<env> TF_VAR_aws_profile=<profile> make apply-refresh

And now we can work with the doctored state as normal, using the `plan` and
`apply` `make` targets.

## Restoring a state backup

Simply identify the backend GCS bucket for the environment and copy the backup
`default.tfstate` stored earlier into it, using `gsutil` or the Google Cloud
Platform Console.
