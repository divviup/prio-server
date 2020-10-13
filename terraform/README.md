# Prio server Terraform module

This Terraform module manages a [GKE cluster](https://cloud.google.com/kubernetes-engine/docs) which hosts a Prio data share processor. We create one cluster and one node pool in each region in which we operate, and then run each PHA's facilitator instance in its own Kubernetes namespace. Each facilitator consists of a [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) that runs a workflow manager.

You will need these tools:

- [terraform](https://learn.hashicorp.com/tutorials/terraform/install-cli): To make Terraform create Google Cloud Platform and Kubernetes resources.
- [gcloud](https://cloud.google.com/sdk/docs/install): For interacting with Google Cloud Platform.
- [kubectl](https://kubernetes.io/docs/tasks/tools/install-kubectl/): To manage the Kubernetes clusters, jobs, pods, etc.

We configure the [GCP Terraform provider ](https://www.terraform.io/docs/providers/google/index.html) to use the credentials file managed by `gcloud`. To ensure you have well-formed credentials available, do `gcloud auth application-default login` and walk through the authentication flow.

We use a [Terraform remote backend](https://www.terraform.io/docs/backends/index.html) to manage the state of the deployment, and the state file resides in a [Google Cloud Storage](https://cloud.google.com/storage/docs) bucket. `terraform/Makefile` is set up to manage the remote state, including creating buckets as needed. To use the `Makefile` targets, set the `ENV` environment variable to something that matches one of the `tfvars` files in `terraform/variables`. For instance, `ENV=demo-gcp make plan` will  source Terraform state from the remote state for `demo-gcp`. Try `ENV=<anything> make help` to get a list of targets.

If you have everything set up correctly, you should be able to...

- `ENV=demo-gcp make plan` to get an idea of what Terraform thinks the world looks like
- `gcloud container clusters list` to list GKE clusters
- `kubectl -n test-pha-1 get pods` to list the pods in namespace `test-pha-1`

If you're having problems, check `gcloud config list` and `kubectl config current-context` to ensure you are using the right configurations for the cluster you are working with.

## New clusters

To add a facilitator to support a new PHA in an existing region, add their PHA name to the `peer_share_processor_names` variable in the relevant `variables/<environment>.tfvars` file. To bring up a whole new cluster, drop a `your-new-environment.tfvars` file in `variables`, fill in the required variables and use `ENV=your-new-environment make apply` to deploy it. Multiple environments may be deployed to the same GCP region.

## kubectl configuration

When instantiating a new GKE cluster, you will want to merge its configuration into your local `~/.kube/config` so that you can use `kubectl` for cluster management. After a successful `apply`, Terraform will emit a `gcloud` invocation that will update your local config file. More generally, `gcloud container clusters get-credentials <YOUR CLUSTER NAME> --region <GCP REGION>"` will do the trick.

## Formatting

Our continuous integration is set up to do basic validation of Terraform files on pull requests. Besides any other testing, make sure to run `terraform fmt --recursive` or you will get build failures!
