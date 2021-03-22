# Prio server Terraform module

This Terraform module manages a [GKE cluster](https://cloud.google.com/kubernetes-engine/docs) which hosts a Prio data share processor. We create one cluster and one node pool in each region in which we operate, and then run each PHA's data share processor instance in its own Kubernetes namespace. Each data share processor consists of a [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) that runs a workflow manager.

You will need these tools:

- [terraform](https://learn.hashicorp.com/tutorials/terraform/install-cli): To make Terraform create Google Cloud Platform and Kubernetes resources.
- [gcloud](https://cloud.google.com/sdk/docs/install): For interacting with Google Cloud Platform.
- [kubectl](https://kubernetes.io/docs/tasks/tools/install-kubectl/): To manage the Kubernetes clusters, jobs, pods, etc.
- [aws-cli](https://aws.amazon.com/cli/): For interacting with Amazon Web Services through Terraform.
- [helm](https://helm.sh/): To install packaged software on Kubernetes.

We configure the [GCP Terraform provider](https://www.terraform.io/docs/providers/google/index.html) to use the credentials file managed by `gcloud`. To ensure you have well-formed credentials available, do `gcloud auth application-default login` and walk through the authentication flow.

We use the [AWS Terraform provider](https://registry.terraform.io/providers/hashicorp/aws/latest/docs) to use the credentials file managed by aws-cli (`aws`). To ensure you have well-formed credentials available, do `aws iam get-user` and make sure it displays your user information. If it doesn't, use `aws configure` to setup your user. You will need to create an [Access Key](https://console.aws.amazon.com/iam/home#/security_credentials) to configure aws.

We currently use the profile `leuswest2` in terraform (this value is defined in `main.tf`). In order for terraform to recognize your credentials, you will need to configure it with `aws configure --profile leuswest2`.

`helm` is not required for creating or maintaining a cluster, but it's still useful to have configured. Once you have helm installed, add the "stable" repository:

```
helm repo add stable https://charts.helm.sh/stable
```

We use a [Terraform remote backend](https://www.terraform.io/docs/backends/index.html) to manage the state of the deployment, and the state file resides in a [Google Cloud Storage](https://cloud.google.com/storage/docs) bucket. `terraform/Makefile` is set up to manage the remote state, including creating buckets as needed. To use the `Makefile` targets, set the `ENV` environment variable to something that matches one of the `tfvars` files in `terraform/variables`. For instance, `ENV=demo-gcp make plan` will  source Terraform state from the remote state for `demo-gcp`. Try `ENV=<anything> make help` to get a list of targets.

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

Once bootstrapped, subsequent deployments should use `ENV=your-new-environment make apply`. Multiple environments may be deployed to the same GCP region.

## Paired test environments

We have support for creating two paired test environments which can exchange validation shares, along with a convincing simulation of ingestion servers and a portal server. To do this, you will need to create two `.tfvars` files, and on top of the usual variables, each must contain a variable like:

    test_peer_environment = {
      env_with_ingestor    = "with-ingestor"
      env_without_ingestor = "without-ingestor"
    }

The values must correspond to the names of the environments you are using. Pick one of them to be the environment with ingestors. From there, you should be able to bring up the two environments like so:

    ENV=with-ingestor make apply-bootstrap
    ENV=without-ingestor make apply-bootstrap

After the successful `apply-bootstrap` you may need to wait several minutes for managed TLS certificates to finish provisioning. Once those are in place, move on to the full deployment:

    ENV=with-ingestor make apply
    ENV=without-ingestor make apply

In your test setup, you might want to exercise reading and writing data to AWS S3 buckets, to simulate interacting with a peer data share processor that runs in AWS. Add `use_aws = true` to the `.tfvars` file for the environment you specified as `env_without_ingestor` and that env's ingestion and peer validation buckets will be created in S3.

## kubectl configuration

When instantiating a new GKE cluster, you will want to merge its configuration into your local `~/.kube/config` so that you can use `kubectl` for cluster management. After a successful `apply`, Terraform will emit a `gcloud` invocation that will update your local config file. More generally, `gcloud container clusters get-credentials <YOUR CLUSTER NAME> --region <GCP REGION>"` will do the trick.

## Formatting

Our continuous integration is set up to do basic validation of Terraform files on pull requests. Besides any other testing, make sure to run `terraform fmt --recursive` or you will get build failures!

## Autoscaling

We use [Kubernetes Horizontal Pod Autoscalers](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/) (HPAs) to automatically control the size of worker pools that process `intake-batch` and `aggregate` tasks. HPAs can manage deployment sizes based on a variety of different metric types. In our case, the metric of interest is the number of undelivered messages in the GCP PubSub subscriptions for the intake and aggregate tasks.

GCP Stackdriver exposes metrics for PubSub, but to make those metrics visible to Kubernetes components, we deploy Google's [Custom Metrics Stackdriver Adapter](https://github.com/GoogleCloudPlatform/k8s-stackdriver/tree/master/custom-metrics-stackdriver-adapter), which is an implementation of the [external.metrics.k8s.io API](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/#support-for-metrics-apis) that exposes Stackdriver metrics (see the `custom_metrics_adapter` module). Now that Kubernetes can see GCP metrics, we can configure HPAs to watch those metrics.

We create an HPA for each Kubernetes Deployment (see the `kubernetes` Terraform module for details). We set `min_replicas` to 3 and `max_replicas` to 50. A minimum of three ensures we have reasonable minimum capacity at all times, while a maximum of 50 ensures that that we can scale up in response to really extraordinary spikes of workload while limiting the risk of flooding the Kubernetes cluster with more pods than it can possibly schedule.

For `intake-batch` HPAs, we set the target value to 50 per replica. This number is chosen on the rough assumption that a single worker should be able to process 50 ingestion batches in the 10 minute interval between `workflow-manager` runs which will schedule new intake tasks. We must use a non-zero value, as otherwise the HPA would immediately schedule the maximum permitted number of replicas to achieve the desired average number of messages per replica. A consequence of this is that the intake-task queue will never truly be empty, since we have configured the HPA to maintain it at a steady state of 3 replicas (the minimum we configure) * 50 messages / replica = 150 messages. The upshot is that if an intake worker exists, it should never be idle, which means we're making good utilization of the time between `workflow-manager` runs.

For `aggregate` HPAs, we set the target value to one message, and hence one aggregation task, per replica. The number of aggregation tasks that will be scheduled at every aggregation window is constant in the number of aggregation IDs for which the ingestion servers submit batches, and is generally far, far smaller than the number of intake tasks (for example, we expect about a thousand intake tasks per hour for an Apple aggregation, but only seven aggregation tasks per eight hours). By instructing the HPA to provision one worker per task, we achieve maximum parallelism given the current implementation of `facilitator`, enabling us to release sum parts to the portal server as quickly as possible.
