# Prio server Terraform module

This Terraform module manages a [GKE cluster](https://cloud.google.com/kubernetes-engine/docs) which hosts a Prio data share processor. We create one cluster and one node pool in each region in which we operate, and then run each PHA's data share processor instance in its own Kubernetes namespace. Each data share processor consists of a [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) that runs a workflow manager.

You will need these tools:

- [terraform](https://learn.hashicorp.com/tutorials/terraform/install-cli): To make Terraform create Google Cloud Platform and Kubernetes resources.
- [gcloud](https://cloud.google.com/sdk/docs/install): For interacting with Google Cloud Platform.
- [kubectl](https://kubernetes.io/docs/tasks/tools/install-kubectl/): To manage the Kubernetes clusters, jobs, pods, etc.
- [aws-cli](https://aws.amazon.com/cli/): For interacting with Amazon Web Services through Terraform.
- [helm](https://helm.sh/): To install packaged software on Kubernetes.

We use a [Terraform remote backend](https://www.terraform.io/docs/backends/index.html) to manage the state of the deployment, and the state file resides in a [Google Cloud Storage](https://cloud.google.com/storage/docs) bucket. `terraform/Makefile` is set up to manage the remote state, including creating buckets as needed. To use the `Makefile` targets, set the `ENV` environment variable to something that matches one of the `tfvars` files in `terraform/variables`. For instance, `ENV=demo-gcp make plan` will source Terraform state from the remote state for `demo-gcp`. Try `ENV=<anything> make help` to get a list of targets.

### Google Cloud Platform credentials

We configure the [GCP Terraform provider](https://www.terraform.io/docs/providers/google/index.html) to use the credentials file managed by `gcloud`. To start with, you will need a GCP user with appropriate permissions in the relevant GCP project. To ensure you have well-formed credentials available for that user, do `gcloud auth application-default login` and walk through the authentication flow. Use the `--project` or `--configuration` parameters as appropriate if you have multiple sets of credentials in play. See `gcloud topics configuration` for more information.

### Amazon Web Services credentials

We use the [AWS Terraform provider](https://registry.terraform.io/providers/hashicorp/aws/latest/docs) to use the credentials file managed by the AWS CLI (`aws`). `main.tf` defines a variable `aws_profile` which needs to match a [named profile](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-profiles.html) configured on your machine. To ensure you have well-formed credentials available, do `aws --profile <your-profile-name> iam get-user` and make sure it displays your user information for the appropriate AWS account. If it doesn't, use `aws --profile <your-profile-name> configure` to setup your user. You will need to create an [Access Key](https://console.aws.amazon.com/iam/home#/security_credentials) to configure `aws`. You can then provide the AWS named profile to Terraform by either setting `aws_profile = <your-profile-name>` in the relevant environment's `.tfvars` file, or setting the `TF_VAR_aws_profile` environment variable when you invoke `Makefile` targets, e.g.:

    ENV=your-dev-env TF_VAR_aws_profile=your-aws-profile make apply

Using `TF_VAR_aws_profile` is preferred because `.tfvars` files are checked into git and AWS profile names need not be consistent on everyone's machines.

#### MFA credentials

The AWS account or user you are using to deploy may have policies configured that require multi-factor authentication in the AWS CLI, and hence for the Terraform AWS provider. If this is the case, you may find that seemingly benign actions like `aws ec2 describe-availability-zones` fail with `UnauthorizedOperation`. You need to obtain session credentials authenticated with MFA using `aws sts get-session-token --serial-number <MFA device ARN> --token-code 123456`, which can then be used for further API calls. See [Amazon documentation](https://aws.amazon.com/premiumsupport/knowledge-center/authenticate-mfa-cli/) for more details.

### Helm repositories

`helm` is not required for creating or maintaining a cluster (we instead use the Helm Terraform provider), but it's still useful to have configured. Once you have helm installed, add the "stable" repository:

```
helm repo add stable https://charts.helm.sh/stable
```

If you have everything set up correctly, you should be able to...

- `ENV=demo-gcp make plan` to get an idea of what Terraform thinks the world looks like
- `gcloud container clusters list` to list GKE clusters
- `kubectl -n test-pha-1 get pods` to list the pods in namespace `test-pha-1`

If you're having problems, check `gcloud config list` and `kubectl config current-context` to ensure you are using the right configurations for the cluster you are working with.

## New clusters

To add a data share processor to support a new locality, add that locality's name to the `localities` variable in the relevant `variables/<environment>.tfvars` file.

To bring up a whole new cluster, drop a `your-new-environment.tfvars` file in `variables`, fill in the required variables and then bootstrap it with:

    ENV=your-new-environment make apply-bootstrap

This will deploy just enough of an environment to permit peers to begin deploying resources. Once your environment is bootstrapped, and once all the other servers you intend to exchange data with have bootstrapped, finish the deploy with

    ENV=your-new-environment make apply

Once bootstrapped, subsequent deployments should use `ENV=your-new-environment make apply`. Multiple environments may be deployed to the same GCP region and project. After running successfully, the `apply` target will instruct you to run `deploy-tool` in order to generate secrets and post specific manifests. `deploy-tool` must be re-run when deploying new localities into an environment to create their secrets and manifests.

## Paired test environments

We have support for creating two paired test environments which can exchange validation shares, along with a convincing simulation of ingestion servers and a portal server. To do this, you will need to create two `.tfvars` files, and on top of the usual variables, each must contain a variable like:

    test_peer_environment = {
      env_with_ingestor    = "with-ingestor"
      env_without_ingestor = "without-ingestor"
    }

The values must correspond to the names of the environments you are using. Pick one of them to be the environment with ingestors. From there, you should be able to bring up the two environments like so:

    ENV=with-ingestor make apply-bootstrap
    ENV=without-ingestor make apply-bootstrap

After the successful `apply-bootstrap`, you will need to run `deploy-tool` against the environment with the ingestors in order to generate global manifests that can later be used to deploy per-locality resources. Additionally, you may need to wait several minutes for managed TLS certificates to finish provisioning. Once those are in place, move on to the full deployment:

    ENV=with-ingestor make apply
    ENV=without-ingestor make apply

Note that you MUST deploy the env which hosts ingestors before the other one as otherwise ingestor global manifests won't be available. After invoking the `apply` target, make sure to run `deploy-tool` to generate secrets and specific manifests.

In your test setup, you might want to exercise reading and writing data to AWS S3 buckets, to simulate interacting with a peer data share processor that runs in AWS. Add `use_aws = true` to the `.tfvars` file for the environment you specified as `env_without_ingestor` and that env's ingestion and peer validation buckets will be created in S3.

## kubectl configuration

When instantiating a new GKE cluster, you will want to merge its configuration into your local `~/.kube/config` so that you can use `kubectl` for cluster management. After a successful `apply`, Terraform will emit a `gcloud` or `aws` invocation that will update your local config file. More generally, `gcloud container clusters get-credentials <YOUR CLUSTER NAME> --region <GCP REGION>"` will do the trick.

## Name collisions

When modifying Terraform modules, pay close attention to the naming of resources. Depending on whether you are managing cloud-level or Kubernetes-level resources, you are claiming unique names in differently scopes. For instance, Kubernetes resources are often [namespaced](https://kubernetes.io/docs/concepts/overview/working-with-objects/namespaces/), and so only need unique names within that narrow scope. However, some Google Cloud Platform resources must have unique names per-project, meaning resources in two different environments deployed to the same GCP project could collide, and some resources like GCS or S3 buckets must have globally unique names. Sufficiently unique names can usually be constructed by interpolating environment name into a resource's name, and it pays to think ahead since some resource names cannot be reclaimed even after the resource is deleted (e.g., S3 buckets).

## Formatting

Our continuous integration is set up to do basic validation of Terraform files on pull requests. Besides any other testing, make sure to run `terraform fmt --recursive` or you will get build failures!

## Secrets backup in GCP

`terraform` and `deploy-tool` conspire to create and populate Kubernetes secrets in which we store batch signing and packet decryption private keys. These Kubernetes secrets are the source of truth since they are used to plumb the keys into containers via environment variables. However, for disaster recovery, we also support backing up Kubernetes secrets into [GCP Secret Manager](https://cloud.google.com/secret-manager/docs) to enable disaster recovery.

If `deploy-tool` is invoked with the `--key-backup-gcp-project` argument, it will iterate over all Kubernetes namespaces and back up the packet decryption key as well as the batch signing keys into GCP Secret Manager, in the specified GCP project, which must already exist and must already have the Secret Manager API enabled. The GCP secret will have a label `kubernetes_uid` whose value will be the Kubernetes secret's `.metadata.uid` field, allowing operators to ensure the correct, current version of a secret is backed up.

The secrets backup is intentionally managed completely outside of Terraform so that it cannot be destroyed or tampered with by an accidental `terraform destroy`. Further, `deploy-tool` never deletes or replaces GCP secrets, and if it detects something unusual it simply tells the human operator about it rather than trying to clean up on its own.

`prio-server` does not currently have any disaster recovery features, but having copies of the keys should at least enable manual reconstruction of a destroyed or damaged cluster without requiring new keys to be exchanged with partners.

## Debugging

Debugging Terraform problems can be tricky since som eproviders emit terse and unhelpful error messages. To get more insight into what is going on, set the environment variable `TF_LOG=debug` when invoking `Makefile` targets like `apply` and Terraform will emit a huge amount of data, usually including HTTP requests to cloud platforms. See [Terraform documentation](https://www.terraform.io/docs/cli/config/environment-variables.html) for more on supported environment variables.
